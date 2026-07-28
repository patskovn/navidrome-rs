//! Root feature: sign-in gate, tabs, and the player bar.

use std::time::Duration;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::app::albums;
use crate::app::auth;
use crate::app::player;
use crate::app::songs;
use crate::audio::AudioEngine;
use crate::config::ServerConfig;
use crate::subsonic::SharedLibrary;

/// Redraw cadence while audio plays, driving the spectrum animation.
///
/// The store notifies observers once per action, so this tick *is* the frame
/// rate. It is cancelled the moment playback stops — otherwise the app would
/// redraw 20 times a second while sitting idle.
const TICK: Duration = Duration::from_millis(50);

/// Cancellation id for the visualiser tick.
#[derive(Hash, PartialEq, Eq, Debug)]
struct TickId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Albums,
    AllMusic,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Albums, Tab::AllMusic];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Albums => "Albums",
            Tab::AllMusic => "All Music",
        }
    }
}

/// Signed-out or signed-in. The library client only exists in the latter, which
/// is why the child features are scoped optionally.
#[derive(Debug, PartialEq, Clone)]
pub enum Session {
    // Both variants are boxed: the sign-in form is three text areas and
    // `Library` is three tabs of state, so either one unboxed would size every
    // `Session` (and therefore every `State`) to the larger of them.
    SignedOut(Box<auth::State<'static>>),
    // Boxed: `Library` holds three tabs' worth of state and dwarfs the
    // sign-in form, so an unboxed enum would size every `Session` to it.
    SignedIn(Box<Library>),
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct Library {
    pub tab: Tab,
    pub albums: albums::State,
    pub songs: songs::State,
}

#[derive(Debug, PartialEq, Clone)]
pub struct State {
    pub session: Session,
    pub player: player::State,
    pub status: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            session: Session::SignedOut(Box::default()),
            player: player::State::new(),
            status: None,
        }
    }
}

impl State {
    /// Starts signed in when a previous session was saved.
    pub fn restored(config: Option<ServerConfig>) -> Self {
        match config {
            Some(_) => Self {
                session: Session::SignedIn(Box::default()),
                ..Default::default()
            },
            None => Self::default(),
        }
    }

    /// Whether the active tab's search field holds the keyboard. While it
    /// does, single-key transport shortcuts are text, not commands.
    pub fn search_focused(&self) -> bool {
        let Some(library) = self.library() else {
            return false;
        };
        let focus = match library.tab {
            Tab::Albums => library.albums.focus,
            Tab::AllMusic => library.songs.focus,
        };
        focus == crate::app::PaneFocus::Search
    }

    fn library_mut(&mut self) -> Option<&mut Library> {
        match &mut self.session {
            Session::SignedIn(library) => Some(library.as_mut()),
            Session::SignedOut(_) => None,
        }
    }

    pub fn library(&self) -> Option<&Library> {
        match &self.session {
            Session::SignedIn(library) => Some(library.as_ref()),
            Session::SignedOut(_) => None,
        }
    }

    fn auth_mut(&mut self) -> Option<&mut auth::State<'static>> {
        match &mut self.session {
            Session::SignedOut(auth) => Some(auth.as_mut()),
            Session::SignedIn(_) => None,
        }
    }

    pub fn auth(&self) -> Option<&auth::State<'static>> {
        match &self.session {
            Session::SignedOut(auth) => Some(auth.as_ref()),
            Session::SignedIn(_) => None,
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Event(Event),
    /// Emitted after a successful sign-in so the library can be built.
    SignedIn(ServerConfig),
    Auth(auth::Action),
    Albums(albums::Action),
    Songs(songs::Action),
    Player(player::Action),
    /// Repaint request while audio plays.
    Tick,
    Quit,
}

pub struct Feature {
    pub library: SharedLibrary,
    pub audio: AudioEngine,
}

impl Feature {
    fn tick_effect(active: bool) -> Effect<Action> {
        if !active {
            return Effect::cancel(TickId);
        }
        Effect::run(|send| async move {
            let mut interval = tokio::time::interval(TICK);
            loop {
                interval.tick().await;
                send.send(Action::Tick);
            }
        })
        .cancellable(TickId)
    }
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        Scope::optional(State::auth_mut, case!(Action::Auth), auth::Feature)
            .on_delegate(case!(auth::Action::Delegated), |_state: &mut State, d| {
                match d {
                    auth::Delegated::SignedIn(config) => Effect::send(Action::SignedIn(config)),
                    auth::Delegated::Noop => Effect::none(),
                }
            })
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.albums),
                    case!(Action::Albums),
                    albums::Feature {
                        library: self.library.clone(),
                    },
                )
                .on_delegate(case!(albums::Action::Delegated), |_s: &mut State, d| match d {
                    albums::Delegated::Play(tracks) => Effect::send(Action::Player(
                        player::Action::SetQueue { tracks, index: 0 },
                    )),
                    albums::Delegated::Noop => Effect::none(),
                }),
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.songs),
                    case!(Action::Songs),
                    songs::Feature {
                        library: self.library.clone(),
                    },
                )
                .on_delegate(case!(songs::Action::Delegated), |_s: &mut State, d| match d {
                    songs::Delegated::Play { tracks, index } => {
                        Effect::send(Action::Player(player::Action::SetQueue { tracks, index }))
                    }
                    songs::Delegated::Noop => Effect::none(),
                }),
            )
            .then(
                Scope::new(
                    |s: &mut State| &mut s.player,
                    case!(Action::Player),
                    player::Feature {
                        library: self.library.clone(),
                        audio: self.audio.clone(),
                    },
                )
                .on_delegate(case!(player::Action::Delegated), |_s: &mut State, d| match d {
                    player::Delegated::PlaybackActive(active) => Self::tick_effect(active),
                }),
            )
            .or_else(|state: &mut State, action| match action {
                Action::Event(e) => reduce_event(state, e),

                Action::SignedIn(_config) => {
                    state.session = Session::SignedIn(Box::default());
                    state.status = None;
                    // Populate the first tab immediately.
                    Effect::send(Action::Albums(albums::Action::Reload))
                }

                // A repaint on its own changes nothing; the store notifies for
                // every processed action, which is the whole point.
                Action::Tick => Effect::none(),

                Action::Quit => Effect::quit(),

                Action::Auth(_)
                | Action::Albums(_)
                | Action::Songs(_)
                | Action::Player(_) => unreachable!("claimed by a scope"),
            })
            .reduce(state, action)
    }
}

/// Global keys, then whatever the focused tab wants.
fn reduce_event(state: &mut State, event: Event) -> Effect<Action> {
    let Event::Key(key) = &event else {
        return Effect::none();
    };
    if key.kind != KeyEventKind::Press {
        return Effect::none();
    }

    // Sign-in owns the keyboard entirely until it succeeds.
    if matches!(state.session, Session::SignedOut(_)) {
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return Effect::send(Action::Quit);
        }
        return Effect::send(Action::Auth(auth::Action::Event(event)));
    }

    // Tab always switches tabs, even from the search field.
    if key.code == KeyCode::Tab {
        if let Some(library) = state.library_mut() {
            let index = Tab::ALL.iter().position(|t| *t == library.tab).unwrap_or(0);
            library.tab = Tab::ALL[(index + 1) % Tab::ALL.len()];
        }
        return load_if_empty(state);
    }
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Effect::send(Action::Quit);
    }

    // Everything else is text while the search field has focus: a bare `q` is
    // a search term, not quit.
    if !state.search_focused() {
        match key.code {
            KeyCode::Char('q') => return Effect::send(Action::Quit),
            KeyCode::Char(' ') => {
                return Effect::send(Action::Player(player::Action::TogglePause));
            }
            KeyCode::Char('n') => return Effect::send(Action::Player(player::Action::Next)),
            KeyCode::Char('p') => return Effect::send(Action::Player(player::Action::Previous)),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                return Effect::send(Action::Player(player::Action::VolumeUp));
            }
            KeyCode::Char('-') => {
                return Effect::send(Action::Player(player::Action::VolumeDown));
            }
            _ => {}
        }
    }

    match state.library().map(|l| l.tab) {
        Some(Tab::Albums) => Effect::send(Action::Albums(albums::Action::Event(event))),
        Some(Tab::AllMusic) => Effect::send(Action::Songs(songs::Action::Event(event))),
        None => Effect::none(),
    }
}

/// Loads a tab's contents the first time it is opened.
fn load_if_empty(state: &mut State) -> Effect<Action> {
    match state.library().map(|l| l.tab) {
        Some(Tab::Albums) if state.library().is_some_and(|l| l.albums.items.items.is_empty()) => {
            Effect::send(Action::Albums(albums::Action::Reload))
        }
        Some(Tab::AllMusic) if state.library().is_some_and(|l| l.songs.items.items.is_empty()) => {
            Effect::send(Action::Songs(songs::Action::Reload))
        }
        _ => Effect::none(),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::audio::TrackCache;
    use crate::subsonic::fake::FakeLibrary;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyEventState;
    use std::sync::Arc;
    use tca::TestStore;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn feature() -> Feature {
        let dir = std::env::temp_dir().join(format!("navidrome-entry-test-{}", std::process::id()));
        let cache = Arc::new(TrackCache::new(&dir, 1024).unwrap());
        let (audio, _events) = AudioEngine::start(cache);
        Feature {
            library: Arc::new(FakeLibrary::default()),
            audio,
        }
    }

    fn signed_in_store() -> TestStore<Feature> {
        TestStore::new(
            feature(),
            State {
                session: Session::SignedIn(Box::default()),
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn tab_cycles_between_the_two_tabs() {
        let mut store = signed_in_store();
        for expected in [Tab::AllMusic, Tab::Albums, Tab::AllMusic] {
            store.send(Action::Event(key(KeyCode::Tab)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.tab = expected;
                }
            });
            store.settle();
            assert_eq!(store.state().library().unwrap().tab, expected);
        }
    }

    /// The search field owns the keyboard on arrival, so single-key transport
    /// shortcuts must be treated as text rather than commands.
    #[tokio::test]
    async fn letters_type_into_search_instead_of_triggering_shortcuts() {
        let mut store = signed_in_store();
        assert!(store.state().search_focused(), "search should start focused");

        for c in ['q', 'n', 'p', ' '] {
            store.send(Action::Event(key(KeyCode::Char(c))), |_| {});
            store.settle();
        }

        assert_eq!(store.state().library().unwrap().albums.query, "qnp ");
        assert!(!store.state().player.is_playing, "space must not have played");
    }

    #[tokio::test]
    async fn arrows_move_focus_between_the_field_and_the_list() {
        let mut store = signed_in_store();
        assert!(store.state().search_focused());

        store.send(Action::Event(key(KeyCode::Down)), |_| {});
        store.settle();
        assert!(!store.state().search_focused(), "Down should enter the list");

        store.send(Action::Event(key(KeyCode::Up)), |_| {});
        store.settle();
        assert!(store.state().search_focused(), "Up should return to the field");
    }

    /// With focus in the list, the shortcuts are commands again.
    #[tokio::test]
    async fn shortcuts_work_once_focus_leaves_the_field() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Down)), |_| {});
        store.settle();

        store.send(Action::Event(key(KeyCode::Char(' '))), |_| {});
        store.settle();
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "space should not have been typed"
        );
    }

    #[tokio::test]
    async fn signing_in_swaps_the_session_and_loads_albums() {
        let mut store = TestStore::new(feature(), State::default());
        assert!(store.state().auth().is_some(), "should start signed out");

        store.send(
            Action::SignedIn(ServerConfig {
                url: "https://patskovn-pi".into(),
                username: "nikita".into(),
            }),
            |s| s.session = Session::SignedIn(Box::default()),
        );

        assert!(store.state().library().is_some(), "should be signed in");
        store.settle();
    }

    #[tokio::test]
    async fn actions_for_a_signed_out_session_are_dropped_not_panicked() {
        // Albums actions can still be in flight when the session resets. The
        // old tgpt code panicked here; `Scope::optional` logs and moves on.
        let mut store = TestStore::new(feature(), State::default());
        store.send(Action::Albums(albums::Action::Reload), |_| {});
        store.settle();
        assert!(store.state().auth().is_some());
    }
}
