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
use crate::app::search;
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
    Search,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Albums, Tab::AllMusic, Tab::Search];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Albums => "Albums",
            Tab::AllMusic => "All Music",
            Tab::Search => "Search",
        }
    }

    fn from_digit(c: char) -> Option<Self> {
        match c {
            '1' => Some(Tab::Albums),
            '2' => Some(Tab::AllMusic),
            '3' => Some(Tab::Search),
            _ => None,
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
    pub search: search::State<'static>,
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
    Search(search::Action),
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
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.search),
                    case!(Action::Search),
                    search::Feature {
                        library: self.library.clone(),
                    },
                )
                .on_delegate(case!(search::Action::Delegated), |_s: &mut State, d| match d {
                    search::Delegated::Play { tracks, index } => {
                        Effect::send(Action::Player(player::Action::SetQueue { tracks, index }))
                    }
                    search::Delegated::Noop => Effect::none(),
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
                | Action::Search(_)
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

    let editing_search = state
        .library()
        .is_some_and(|l| l.tab == Tab::Search && l.search.editing);

    // While typing a query, only Ctrl-C escapes: a bare `q` is a search term.
    if !editing_search {
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
            KeyCode::Char(c) if Tab::from_digit(c).is_some() => {
                let tab = Tab::from_digit(c).expect("checked above");
                if let Some(library) = state.library_mut() {
                    library.tab = tab;
                }
                // Load the tab's contents the first time it is opened.
                return match tab {
                    Tab::Albums if state.library().is_some_and(|l| l.albums.albums.items.is_empty()) => {
                        Effect::send(Action::Albums(albums::Action::Reload))
                    }
                    Tab::AllMusic if state.library().is_some_and(|l| l.songs.tracks.items.is_empty()) => {
                        Effect::send(Action::Songs(songs::Action::Reload))
                    }
                    _ => Effect::none(),
                };
            }
            KeyCode::Tab => {
                if let Some(library) = state.library_mut() {
                    let index = Tab::ALL.iter().position(|t| *t == library.tab).unwrap_or(0);
                    library.tab = Tab::ALL[(index + 1) % Tab::ALL.len()];
                }
                return Effect::none();
            }
            _ => {}
        }
    }

    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Effect::send(Action::Quit);
    }

    match state.library().map(|l| l.tab) {
        Some(Tab::Albums) => Effect::send(Action::Albums(albums::Action::Event(event))),
        Some(Tab::AllMusic) => Effect::send(Action::Songs(songs::Action::Event(event))),
        Some(Tab::Search) => Effect::send(Action::Search(search::Action::Event(event))),
        None => Effect::none(),
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
    async fn digits_switch_tabs() {
        let mut store = signed_in_store();

        store.send(Action::Event(key(KeyCode::Char('2'))), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.tab = Tab::AllMusic;
            }
        });
        store.settle();
        assert_eq!(store.state().library().unwrap().tab, Tab::AllMusic);
    }

    #[tokio::test]
    async fn tab_cycles_through_every_tab() {
        let mut store = signed_in_store();
        for expected in [Tab::AllMusic, Tab::Search, Tab::Albums] {
            store.send(Action::Event(key(KeyCode::Tab)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.tab = expected;
                }
            });
            store.settle();
        }
        assert_eq!(store.state().library().unwrap().tab, Tab::Albums);
    }

    #[tokio::test]
    async fn space_toggles_playback() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Char(' '))), |_| {});
        // Nothing is queued, so the toggle is a no-op rather than a crash.
        store.settle();
        assert!(!store.state().player.is_playing);
    }

    #[tokio::test]
    async fn q_quits_but_not_while_typing_a_search() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Char('3'))), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.tab = Tab::Search;
            }
        });
        store.settle();

        // `/` is routed to the search feature, which is what flips `editing`;
        // the root reducer only forwards it, so settle before asserting.
        store.send(Action::Event(key(KeyCode::Char('/'))), |_| {});
        store.settle();
        assert!(
            store.state().library().unwrap().search.editing,
            "pressing / should focus the query field"
        );

        // Now `q` must reach the text field rather than quitting the app.
        store.send(Action::Event(key(KeyCode::Char('q'))), |_| {});
        assert!(
            store.pending_len() > 0,
            "the key should have been routed to search, not swallowed as quit"
        );
        store.settle();
        assert!(
            store.state().library().unwrap().search.editing,
            "still editing after typing a letter"
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
