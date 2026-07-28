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
use std::sync::Arc;

use crate::audio::AudioEngine;
use crate::config::ServerConfig;
use crate::subsonic::SharedLibrary;
use crate::subsonic::SwappableLibrary;

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

/// Output picker, open over whatever tab is showing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DevicePicker {
    pub selected: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub struct State {
    pub help: bool,
    pub picker: Option<DevicePicker>,
    pub session: Session,
    pub player: player::State,
    pub status: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            help: false,
            picker: None,
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
    /// Open or close the output picker.
    TogglePicker,
    /// Open or close the keybinding help.
    ToggleHelp,
    Quit,
}

pub struct Feature {
    /// Held as the switch, not a plain handle, so signing in can install a
    /// real client in place of the signed-out stub.
    pub library: Arc<SwappableLibrary>,
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
        let switch = self.library.clone();

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
                        library: self.library.clone() as SharedLibrary,
                    },
                )
                .on_delegate(case!(albums::Action::Delegated), |_s: &mut State, d| match d {
                    albums::Delegated::Play { tracks, index } => Effect::send(Action::Player(
                        player::Action::SetQueue { tracks, index },
                    )),
                    albums::Delegated::Noop => Effect::none(),
                }),
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.songs),
                    case!(Action::Songs),
                    songs::Feature {
                        library: self.library.clone() as SharedLibrary,
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
                        library: self.library.clone() as SharedLibrary,
                        audio: self.audio.clone(),
                    },
                )
                .on_delegate(case!(player::Action::Delegated), |_s: &mut State, d| match d {
                    player::Delegated::PlaybackActive(active) => Self::tick_effect(active),
                }),
            )
            .or_else(move |state: &mut State, action| match action {
                Action::Event(e) => reduce_event(state, e),

                Action::SignedIn(config) => {
                    // The client is built once at startup, so without this the
                    // signed-out stub would stay installed and every request
                    // would fail until the app was restarted.
                    match crate::subsonic::OpenSubsonicLibrary::new(
                        &config.url,
                        &config.username,
                        &config.password(),
                    ) {
                        Ok(library) => switch.set(Arc::new(library) as SharedLibrary),
                        Err(err) => {
                            state.status = Some(format!("{err:#}"));
                            return Effect::none();
                        }
                    }
                    state.session = Session::SignedIn(Box::default());
                    state.status = None;
                    // Populate the first tab immediately.
                    Effect::send(Action::Albums(albums::Action::Reload))
                }

                // A repaint on its own changes nothing; the store notifies for
                // every processed action, which is the whole point.
                Action::Tick => Effect::none(),

                Action::ToggleHelp => {
                    state.help = !state.help;
                    Effect::none()
                }

                Action::TogglePicker => {
                    state.picker = match state.picker {
                        Some(_) => None,
                        None => Some(DevicePicker::default()),
                    };
                    // Refresh on open: devices come and go while we run.
                    if state.picker.is_some() {
                        return Effect::send(Action::Player(player::Action::Engine(
                            crate::audio::Event::Devices {
                                names: crate::audio::engine::output_devices(),
                                active: state.player.active_device.clone(),
                            },
                        )));
                    }
                    Effect::none()
                }

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

    // Help is read-only, so any key dismisses it rather than only Esc.
    if state.help {
        state.help = false;
        return Effect::none();
    }

    // The picker owns the keyboard while it is open.
    if let Some(picker) = &mut state.picker {
        let count = state.player.devices.len() + 1; // +1 for "system default"
        return match key.code {
            KeyCode::Esc => {
                state.picker = None;
                Effect::none()
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                state.picker = None;
                Effect::none()
            }
            KeyCode::Down | KeyCode::Char('j') => {
                picker.selected = (picker.selected + 1).min(count.saturating_sub(1));
                Effect::none()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.selected = picker.selected.saturating_sub(1);
                Effect::none()
            }
            KeyCode::Enter => {
                let index = picker.selected;
                state.picker = None;
                // Index 0 is the system default; the rest are named devices.
                let name = index
                    .checked_sub(1)
                    .and_then(|i| state.player.devices.get(i))
                    .map(|(id, _)| id.clone());
                Effect::send(Action::Player(player::Action::UseDevice(name)))
            }
            _ => Effect::none(),
        };
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
    // Ctrl-O, not a bare `o`: the search field has focus by default and takes
    // every printable key, so a plain letter would only ever be typed.
    if key.code == KeyCode::Char('o') && key.modifiers == KeyModifiers::CONTROL {
        return Effect::send(Action::TogglePicker);
    }
    // `?` is reserved even though the search field takes every other printable
    // key. Searching for a literal question mark is vanishingly rare, and `?`
    // for help is worth more than that.
    if key.code == KeyCode::Char('?') {
        return Effect::send(Action::ToggleHelp);
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

    fn audio() -> AudioEngine {
        let dir = std::env::temp_dir().join(format!("navidrome-entry-test-{}", std::process::id()));
        let cache = Arc::new(TrackCache::new(&dir, 1024).unwrap());
        let (audio, _events) = AudioEngine::start(cache);
        audio
    }

    fn feature() -> Feature {
        let (audio, _events) = {
            let dir =
                std::env::temp_dir().join(format!("navidrome-entry-test-{}", std::process::id()));
            let cache = Arc::new(TrackCache::new(&dir, 1024).unwrap());
            AudioEngine::start(cache)
        };
        Feature {
            library: Arc::new(SwappableLibrary::new(Arc::new(FakeLibrary::default()))),
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

    fn ctrl(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    /// Regression: the picker key was a bare `o`, which the search field --
    /// focused by default -- simply typed, making the picker unreachable.
    #[tokio::test]
    async fn the_picker_opens_from_the_search_field() {
        let mut store = signed_in_store();
        assert!(store.state().search_focused(), "search starts focused");

        store.send(Action::Event(ctrl(KeyCode::Char('o'))), |_| {});
        store.settle();
        assert!(store.state().picker.is_some(), "Ctrl-O should open the picker");
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "and must not have been typed into the query"
        );

        store.send(Action::Event(key(KeyCode::Esc)), |s| s.picker = None);
        assert!(store.state().picker.is_none(), "Esc should close it");
    }

    #[tokio::test]
    async fn question_mark_opens_help_from_the_search_field() {
        let mut store = signed_in_store();
        assert!(store.state().search_focused());

        store.send(Action::Event(key(KeyCode::Char('?'))), |_| {});
        store.settle();
        assert!(store.state().help, "? should open help");
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "and must not be typed into the query"
        );
    }

    #[tokio::test]
    async fn any_key_dismisses_help() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Char('?'))), |_| {});
        store.settle();
        assert!(store.state().help);

        // Help is read-only, so it should not need a specific key to leave.
        store.send(Action::Event(key(KeyCode::Char('x'))), |s| s.help = false);
        assert!(!store.state().help, "any key should close help");
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "the dismissing key should be swallowed, not typed"
        );
    }

    /// A bare `o` stays search text.
    #[tokio::test]
    async fn a_plain_o_is_typed_not_a_shortcut() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Char('o'))), |_| {});
        store.settle();
        assert!(store.state().picker.is_none());
        assert_eq!(store.state().library().unwrap().albums.query, "o");
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
                password: "hunter2".into(),
            }),
            |s| s.session = Session::SignedIn(Box::default()),
        );

        assert!(store.state().library().is_some(), "should be signed in");
        store.settle();
    }

    /// Regression: signing in used to flip the UI but leave the signed-out
    /// stub installed, so every request failed until the app was restarted.
    #[tokio::test]
    async fn signing_in_installs_a_working_client() {
        use crate::subsonic::Library as _;

        let switch = Arc::new(SwappableLibrary::new(Arc::new(NotSignedInStub)));
        let feature = Feature {
            library: switch.clone(),
            audio: audio(),
        };
        let mut store = TestStore::new(feature, State::default());

        // Before sign-in the stub refuses everything.
        assert!(switch.ping().await.is_err());

        // A real client is swapped in behind the switch.
        switch.set(Arc::new(FakeLibrary::default()));
        assert!(
            switch.ping().await.is_ok(),
            "the switch should route to the new client"
        );

        store.send(
            Action::SignedIn(ServerConfig {
                url: "https://patskovn-pi".into(),
                username: "nikita".into(),
                password: "hunter2".into(),
            }),
            |s| s.session = Session::SignedIn(Box::default()),
        );
        store.settle();

        // The switch now holds a real client. It still cannot reach that server
        // from a test, but it fails as a client rather than as the stub — which
        // is exactly the distinction the bug got wrong.
        let err = format!("{:#}", switch.ping().await.unwrap_err());
        assert!(
            !err.contains("not signed in"),
            "the signed-out stub is still installed: {err}"
        );
    }

    /// Stands in for `main::NotSignedIn`, which lives in the binary root.
    struct NotSignedInStub;

    #[async_trait::async_trait]
    impl crate::subsonic::Library for NotSignedInStub {
        async fn ping(&self) -> anyhow::Result<()> {
            anyhow::bail!("not signed in")
        }
        async fn albums(
            &self,
            _: crate::subsonic::AlbumSort,
            _: u32,
            _: u32,
        ) -> anyhow::Result<Vec<crate::subsonic::Album>> {
            anyhow::bail!("not signed in")
        }
        async fn album_tracks(&self, _: &str) -> anyhow::Result<Vec<crate::subsonic::Track>> {
            anyhow::bail!("not signed in")
        }
        async fn all_tracks(
            &self,
            _: u32,
            _: u32,
        ) -> anyhow::Result<Vec<crate::subsonic::Track>> {
            anyhow::bail!("not signed in")
        }
        async fn search(&self, _: &str) -> anyhow::Result<crate::subsonic::SearchResults> {
            anyhow::bail!("not signed in")
        }
        async fn cover_art(&self, _: &str) -> anyhow::Result<Vec<u8>> {
            anyhow::bail!("not signed in")
        }
        fn stream_url(&self, _: &str) -> anyhow::Result<String> {
            anyhow::bail!("not signed in")
        }
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
