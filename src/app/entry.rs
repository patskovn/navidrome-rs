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
use crate::app::artists;
use crate::app::auth;
use crate::app::player;
use crate::app::radios;
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
    Artists,
    AllMusic,
    Radios,
    Queue,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Albums,
        Tab::Artists,
        Tab::AllMusic,
        Tab::Radios,
        Tab::Queue,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Albums => "Albums",
            Tab::Artists => "Artists",
            Tab::AllMusic => "All Music",
            Tab::Radios => "Radios",
            Tab::Queue => "Queue",
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
    pub artists: artists::State,
    pub songs: songs::State,
    pub radios: radios::State,
    /// Cursor for the Queue tab.
    ///
    /// Not a scoped child feature like the other two tabs: the queue itself
    /// lives in `player::State`, and every key here needs its length to clamp
    /// against. A child reducer would only see the cursor, so it would have to
    /// be handed a copy of the length to keep in sync — two sources of truth
    /// for one number.
    pub queue_cursor: usize,
}

/// Output picker, open over whatever tab is showing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DevicePicker {
    pub selected: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub struct State {
    pub help: bool,
    /// Whether the pixel-art scene fills the screen in place of the tabs.
    pub scene: bool,
    /// How many scenes along from the one the music picked, while the scene
    /// is open. Reset on closing, so the music picks again next time.
    pub scene_turn: i32,
    pub picker: Option<DevicePicker>,
    pub session: Session,
    pub player: player::State,
    pub status: Option<String>,
    /// Whether the terminal window itself has focus, which dims the accent.
    ///
    /// Starts true: a terminal that does not report focus never sends the
    /// events, and the app should look normal there rather than permanently
    /// faint.
    pub focused: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            help: false,
            scene: false,
            scene_turn: 0,
            picker: None,
            session: Session::SignedOut(Box::default()),
            player: player::State::new(),
            status: None,
            focused: true,
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
            Tab::Artists => library.artists.focus,
            Tab::AllMusic => library.songs.focus,
            // No search field here, so single-key shortcuts stay commands.
            Tab::Queue => return false,
            // Text only while adding a station, or answering a delete prompt.
            Tab::Radios => return library.radios.captures_keys(),
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
    Artists(artists::Action),
    Songs(songs::Action),
    Radios(radios::Action),
    Player(player::Action),
    /// Repaint request while audio plays.
    Tick,
    /// Open or close the output picker.
    TogglePicker,
    /// Open or close the keybinding help.
    ToggleHelp,
    /// Show or hide the fullscreen pixel-art scene.
    ToggleScene,
    Quit,
}

pub struct Feature {
    /// Held as the switch, not a plain handle, so signing in can install a
    /// real client in place of the signed-out stub.
    pub library: Arc<SwappableLibrary>,
    pub audio: AudioEngine,
    /// Checks a station URL before the add form previews it.
    pub probe: crate::probe::SharedProbe,
    /// Where the Radios tab searches for stations.
    pub directory: crate::directory::SharedDirectory,
}

impl Feature {
    fn tick_effect(active: bool) -> Effect<Action> {
        // Whatever tick is running stops either way. This is asked for again
        // while it runs — the scene opened during a song — and a second
        // ticker beside the first would double the frame rate, a third
        // triple it, for as long as the song lasted.
        let stop = Effect::cancel(TickId);
        if !active {
            return stop;
        }
        let tick = Effect::run(|send| async move {
            let mut interval = tokio::time::interval(TICK);
            loop {
                interval.tick().await;
                send.send(Action::Tick);
            }
        })
        .cancellable(TickId);
        // In this order. Merged effects run as written, so the cancel reaches
        // the old tick and not the new. (tca-rs 0.2.0 ran them backwards.)
        Effect::merge([stop, tick])
    }
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        let switch = self.library.clone();

        Scope::optional(State::auth_mut, case!(Action::Auth), auth::Feature)
            .on_delegate(
                case!(auth::Action::Delegated),
                |_state: &mut State, d| match d {
                    auth::Delegated::SignedIn(config) => Effect::send(Action::SignedIn(config)),
                    auth::Delegated::Noop => Effect::none(),
                },
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.albums),
                    case!(Action::Albums),
                    albums::Feature {
                        library: self.library.clone() as SharedLibrary,
                    },
                )
                .on_delegate(
                    case!(albums::Action::Delegated),
                    |_s: &mut State, d| match d {
                        albums::Delegated::Play { tracks, index } => {
                            Effect::send(Action::Player(player::Action::SetQueue { tracks, index }))
                        }
                        albums::Delegated::Noop => Effect::none(),
                    },
                ),
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.artists),
                    case!(Action::Artists),
                    artists::Feature {
                        library: self.library.clone() as SharedLibrary,
                    },
                )
                .on_delegate(
                    case!(artists::Action::Delegated),
                    |_s: &mut State, d| match d {
                        artists::Delegated::Play { tracks, index } => {
                            Effect::send(Action::Player(player::Action::SetQueue { tracks, index }))
                        }
                        artists::Delegated::Noop => Effect::none(),
                    },
                ),
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.songs),
                    case!(Action::Songs),
                    songs::Feature {
                        library: self.library.clone() as SharedLibrary,
                    },
                )
                .on_delegate(
                    case!(songs::Action::Delegated),
                    |_s: &mut State, d| match d {
                        songs::Delegated::Play { tracks, index } => {
                            Effect::send(Action::Player(player::Action::SetQueue { tracks, index }))
                        }
                        songs::Delegated::Noop => Effect::none(),
                    },
                ),
            )
            .then(
                Scope::optional(
                    |s: &mut State| s.library_mut().map(|l| &mut l.radios),
                    case!(Action::Radios),
                    radios::Feature {
                        library: self.library.clone() as SharedLibrary,
                        probe: self.probe.clone(),
                        directory: self.directory.clone(),
                    },
                )
                .on_delegate(
                    case!(radios::Action::Delegated),
                    |s: &mut State, d| match d {
                        radios::Delegated::Play(track) => {
                            Effect::send(Action::Player(player::Action::SetQueue {
                                tracks: vec![*track],
                                index: 0,
                            }))
                        }
                        // Only a preview: by now the user may have started
                        // something else, and that must not be cut off.
                        radios::Delegated::StopPreview => {
                            let previewing = s
                                .player
                                .current_track()
                                .is_some_and(|t| t.id == radios::PREVIEW_ID);
                            if previewing {
                                Effect::send(Action::Player(player::Action::Stop))
                            } else {
                                Effect::none()
                            }
                        }
                    },
                ),
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
                .on_delegate(
                    case!(player::Action::Delegated),
                    |s: &mut State, d| match d {
                        // The scene animates on the same tick, so it keeps the
                        // tick alive even when the music stops.
                        player::Delegated::PlaybackActive(active) => {
                            Self::tick_effect(active || s.scene)
                        }
                    },
                ),
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

                Action::ToggleScene => {
                    state.scene = !state.scene;
                    state.scene_turn = 0;
                    // The tick is the scene's frame rate. Closing it stops the
                    // tick only if nothing is playing to need it.
                    Self::tick_effect(state.scene || state.player.is_playing)
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
                | Action::Artists(_)
                | Action::Songs(_)
                | Action::Radios(_)
                | Action::Player(_) => unreachable!("claimed by a scope"),
            })
            .reduce(state, action)
    }
}

/// Global keys, then whatever the focused tab wants.
fn reduce_event(state: &mut State, event: Event) -> Effect<Action> {
    // Window focus, which the accent follows. Reported only by terminals that
    // support focus tracking; where they do not, `focused` stays true.
    if let Event::FocusGained | Event::FocusLost = event {
        state.focused = matches!(event, Event::FocusGained);
        return Effect::none();
    }

    // A paste is text, and only the station form takes text as a whole.
    if let Event::Paste(_) = &event {
        if state.scene {
            return Effect::none();
        }
        return match state.library().map(|l| (l.tab, l.radios.takes_text())) {
            Some((Tab::Radios, true)) => Effect::send(Action::Radios(radios::Action::Event(event))),
            _ => Effect::none(),
        };
    }

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

    // Ctrl-V shows the scene from anywhere, a text field included; a bare `v`
    // does too, below, wherever letters are not text.
    if key.code == KeyCode::Char('v') && key.modifiers == KeyModifiers::CONTROL {
        return Effect::send(Action::ToggleScene);
    }
    // The scene covers the tabs, so their keys have nowhere to go. Transport,
    // help, the output picker and quit still work below.
    if state.scene {
        match key.code {
            KeyCode::Esc | KeyCode::Char('v') => return Effect::send(Action::ToggleScene),
            // Through the scenes, either way; Tab, with no tabs to switch
            // while the scene covers them, goes forward too.
            KeyCode::Right | KeyCode::Tab => {
                state.scene_turn += 1;
                return Effect::none();
            }
            KeyCode::Left => {
                state.scene_turn -= 1;
                return Effect::none();
            }
            _ => {}
        }
    }

    // Tab always switches tabs, even from the search field.
    if key.code == KeyCode::Tab {
        let sounding = state.player.current;
        if let Some(library) = state.library_mut() {
            let index = Tab::ALL.iter().position(|t| *t == library.tab).unwrap_or(0);
            library.tab = Tab::ALL[(index + 1) % Tab::ALL.len()];
            // Arriving at the queue, start on whatever is playing — that is
            // where you are most likely to want to act.
            if library.tab == Tab::Queue {
                library.queue_cursor = sounding.unwrap_or(0);
            }
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
    //
    // The station form is the exception: stream URLs carry query strings
    // (`/proxy/jamz?mp=/1`), so there `?` is text like any other key.
    let typing_url = state
        .library()
        .is_some_and(|l| l.tab == Tab::Radios && l.radios.takes_text());
    if key.code == KeyCode::Char('?') && !typing_url {
        return Effect::send(Action::ToggleHelp);
    }

    // Everything else is text while the search field has focus: a bare `q` is
    // a search term, not quit. Under the scene no field is visible, so single
    // keys are commands there.
    if !state.search_focused() || state.scene {
        match key.code {
            KeyCode::Char('q') => return Effect::send(Action::Quit),
            KeyCode::Char('v') => return Effect::send(Action::ToggleScene),
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

    if state.scene {
        return Effect::none();
    }

    match state.library().map(|l| l.tab) {
        Some(Tab::Albums) => Effect::send(Action::Albums(albums::Action::Event(event))),
        Some(Tab::Artists) => Effect::send(Action::Artists(artists::Action::Event(event))),
        Some(Tab::AllMusic) => Effect::send(Action::Songs(songs::Action::Event(event))),
        Some(Tab::Radios) => Effect::send(Action::Radios(radios::Action::Event(event))),
        Some(Tab::Queue) => reduce_queue_key(state, key.code),
        None => Effect::none(),
    }
}

/// Keys for the Queue tab.
///
/// It has no search field, so unlike the browse tabs a bare letter is a command
/// here rather than text. `q` still quits — that is handled above, before this
/// is reached.
fn reduce_queue_key(state: &mut State, code: KeyCode) -> Effect<Action> {
    let len = state.player.queue.len();
    let Some(library) = state.library_mut() else {
        return Effect::none();
    };

    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            library.queue_cursor = (library.queue_cursor + 1).min(len.saturating_sub(1));
            Effect::none()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            library.queue_cursor = library.queue_cursor.saturating_sub(1);
            Effect::none()
        }
        KeyCode::Enter => {
            let index = library.queue_cursor;
            if index >= len {
                return Effect::none();
            }
            Effect::send(Action::Player(player::Action::JumpTo(index)))
        }
        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
            let index = library.queue_cursor;
            if index >= len {
                return Effect::none();
            }
            // Keep the cursor on a real row once the queue is one shorter. The
            // player reducer owns the removal; only the cursor is ours.
            library.queue_cursor = index.min(len.saturating_sub(2));
            Effect::send(Action::Player(player::Action::Remove(index)))
        }
        _ => Effect::none(),
    }
}

/// Loads a tab's contents the first time it is opened.
fn load_if_empty(state: &mut State) -> Effect<Action> {
    match state.library().map(|l| l.tab) {
        Some(Tab::Albums)
            if state
                .library()
                .is_some_and(|l| l.albums.items.items.is_empty()) =>
        {
            Effect::send(Action::Albums(albums::Action::Reload))
        }
        // `loaded`, not emptiness: a server with no artists would otherwise be
        // asked again on every visit. `loading` keeps a quick Tab-around from
        // firing a second request while the first is out.
        Some(Tab::Artists)
            if state
                .library()
                .is_some_and(|l| !l.artists.loaded && !l.artists.loading) =>
        {
            Effect::send(Action::Artists(artists::Action::Reload))
        }
        Some(Tab::AllMusic)
            if state
                .library()
                .is_some_and(|l| l.songs.items.items.is_empty()) =>
        {
            Effect::send(Action::Songs(songs::Action::Reload))
        }
        Some(Tab::Radios)
            if state
                .library()
                .is_some_and(|l| !l.radios.loaded && !l.radios.loading) =>
        {
            Effect::send(Action::Radios(radios::Action::Reload))
        }
        // The queue is already in memory; there is nothing to fetch.
        Some(Tab::Queue) => Effect::none(),
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
            probe: Arc::new(crate::probe::fake::FakeProbe::failing(
                "no network in tests",
            )),
            directory: Arc::new(crate::directory::fake::FakeDirectory::default()),
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
    async fn tab_cycles_through_every_tab() {
        let mut store = signed_in_store();
        for expected in [
            Tab::Artists,
            Tab::AllMusic,
            Tab::Radios,
            Tab::Queue,
            Tab::Albums,
            Tab::Artists,
        ] {
            store.send(Action::Event(key(KeyCode::Tab)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.tab = expected;
                }
            });
            store.settle();
            assert_eq!(store.state().library().unwrap().tab, expected);
        }
    }

    /// The roster is fetched on the first visit only — including when the
    /// server has no artists, which must not be re-asked on every visit.
    #[tokio::test]
    async fn arriving_at_artists_loads_the_roster_once() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Tab)), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.tab = Tab::Artists;
            }
        });
        assert_eq!(store.pending_len(), 1, "arriving should ask for the roster");
        store.settle();
        assert!(store.state().library().unwrap().artists.loaded);

        // Round the tabs and back: an empty roster is still a loaded one.
        for _ in 0..Tab::ALL.len() {
            store.send(Action::Event(key(KeyCode::Tab)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    let i = Tab::ALL.iter().position(|t| *t == l.tab).unwrap();
                    l.tab = Tab::ALL[(i + 1) % Tab::ALL.len()];
                }
            });
            if store.state().library().unwrap().tab == Tab::Artists {
                assert_eq!(store.pending_len(), 0, "the roster was fetched again");
            }
            store.settle();
        }
        assert_eq!(store.state().library().unwrap().tab, Tab::Artists);
    }

    fn radio_form_store(
        field: radios::FormField,
        player: player::State,
        previewing: bool,
    ) -> TestStore<Feature> {
        TestStore::new(
            feature(),
            State {
                session: Session::SignedIn(Box::new(Library {
                    tab: Tab::Radios,
                    radios: radios::State {
                        loaded: true,
                        form: Some(radios::Form {
                            field,
                            previewing,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                })),
                player,
                ..Default::default()
            },
        )
    }

    fn radio_form(store: &TestStore<Feature>) -> Option<&radios::Form> {
        store.state().library().unwrap().radios.form.as_ref()
    }

    /// While adding a station every key is text — including the ones that are
    /// shortcuts elsewhere, and `?`, which stream URLs are full of.
    #[tokio::test]
    async fn the_station_form_takes_shortcut_keys_as_text() {
        let mut store = radio_form_store(radios::FormField::Name, player::State::new(), false);
        for c in ['q', '?', 'n', ' ', 'p'] {
            store.send(Action::Event(key(KeyCode::Char(c))), |_| {});
            store.settle();
        }
        assert_eq!(radio_form(&store).unwrap().name, "q?n p");
        assert!(!store.state().help, "? must not have opened help");
    }

    /// The directory search is a text field too: `q` searches, `?` is text.
    #[tokio::test]
    async fn the_station_search_takes_shortcut_keys_as_text() {
        let mut store = TestStore::new(
            feature(),
            State {
                session: Session::SignedIn(Box::new(Library {
                    tab: Tab::Radios,
                    radios: radios::State {
                        loaded: true,
                        search: Some(Default::default()),
                        ..Default::default()
                    },
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        for c in ['q', '?'] {
            store.send(Action::Event(key(KeyCode::Char(c))), |_| {});
            store.settle();
        }
        let search = store.state().library().unwrap().radios.search.clone();
        assert_eq!(search.unwrap().query, "q?");
        assert!(!store.state().help);
    }

    /// A URL is usually pasted, and arrives as one event.
    #[tokio::test]
    async fn a_pasted_url_lands_in_the_field() {
        let mut store = radio_form_store(radios::FormField::Url, player::State::new(), false);
        store.send(
            Action::Event(Event::Paste(
                "https://x.radio.invalid/proxy/jamz?mp=/1\n".into(),
            )),
            |_| {},
        );
        store.settle();
        assert_eq!(
            radio_form(&store).unwrap().url,
            "https://x.radio.invalid/proxy/jamz?mp=/1"
        );
    }

    fn playing(track: crate::subsonic::Track) -> player::State {
        player::State {
            queue: vec![track],
            current: Some(0),
            is_playing: true,
            ..player::State::new()
        }
    }

    /// Leaving the form stops its preview…
    #[tokio::test]
    async fn leaving_the_form_stops_its_preview() {
        let preview = crate::subsonic::Track {
            id: radios::PREVIEW_ID.into(),
            live_url: Some("https://x.radio.invalid/live".into()),
            ..Default::default()
        };
        let mut store = radio_form_store(radios::FormField::Url, playing(preview), true);
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.settle();
        assert!(radio_form(&store).is_none());
        assert_eq!(store.state().player.current, None, "the preview stopped");
    }

    /// …but only the preview. If the user has started something else since,
    /// that keeps playing.
    #[tokio::test]
    async fn leaving_the_form_spares_other_playback() {
        let album_track = crate::subsonic::Track {
            id: "song-1".into(),
            ..Default::default()
        };
        let mut store = radio_form_store(radios::FormField::Url, playing(album_track), true);
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.settle();
        assert!(radio_form(&store).is_none());
        assert_eq!(store.state().player.current, Some(0), "the song plays on");
    }

    fn scene_open(store: &TestStore<Feature>) -> bool {
        store.state().scene
    }

    /// `v` is a letter while a field has the keyboard; Ctrl-V works anyway.
    #[tokio::test]
    async fn the_scene_opens_with_ctrl_v_even_from_a_field() {
        let mut store = signed_in_store();
        assert!(store.state().search_focused());

        store.send(Action::Event(key(KeyCode::Char('v'))), |_| {});
        store.settle();
        assert!(!scene_open(&store), "a plain v in the field is text");
        assert_eq!(store.state().library().unwrap().albums.query, "v");

        store.send(Action::Event(ctrl(KeyCode::Char('v'))), |_| {});
        store.settle();
        assert!(scene_open(&store));
    }

    #[tokio::test]
    async fn a_bare_v_toggles_the_scene_outside_a_field() {
        let mut store = signed_in_store();
        store.send(Action::Event(key(KeyCode::Down)), |_| {});
        store.settle();
        assert!(!store.state().search_focused());

        store.send(Action::Event(key(KeyCode::Char('v'))), |_| {});
        store.settle();
        assert!(scene_open(&store));
        store.send(Action::Event(key(KeyCode::Char('v'))), |_| {});
        store.settle();
        assert!(!scene_open(&store), "v again closes it");
    }

    /// Under the scene, keys meant for the hidden tab go nowhere — nothing is
    /// typed into a search you cannot see, and Tab does not move it.
    #[tokio::test]
    async fn the_scene_swallows_tab_keys_and_closes_on_escape() {
        let mut store = signed_in_store();
        store.send(Action::Event(ctrl(KeyCode::Char('v'))), |_| {});
        store.settle();

        // Tab turns the scene now; it must still not switch the tab beneath.
        for code in [
            KeyCode::Char('x'),
            KeyCode::Tab,
            KeyCode::Down,
            KeyCode::Backspace,
        ] {
            store.send(Action::Event(key(code)), |s| {
                if code == KeyCode::Tab {
                    s.scene_turn += 1;
                }
            });
            store.settle();
        }
        let library = store.state().library().unwrap();
        assert_eq!(library.albums.query, "", "nothing typed underneath");
        assert_eq!(library.tab, Tab::Albums, "Tab did nothing");
        assert!(scene_open(&store));

        store.send(Action::Event(Event::Paste("hidden".into())), |_| {});
        store.settle();

        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.settle();
        assert!(!scene_open(&store), "Esc leaves the scene");
    }

    /// ← and → (and Tab) turn through the scenes while one is open; closing
    /// forgets the turn, so the music chooses again next time.
    #[tokio::test]
    async fn arrows_turn_through_the_scenes_and_closing_resets() {
        let mut store = signed_in_store();
        store.send(Action::Event(ctrl(KeyCode::Char('v'))), |_| {});
        store.settle();

        for (code, turn) in [
            (KeyCode::Right, 1),
            (KeyCode::Tab, 2),
            (KeyCode::Left, 1),
            (KeyCode::Left, 0),
            (KeyCode::Left, -1),
        ] {
            store.send(Action::Event(key(code)), |s| s.scene_turn = turn);
        }
        assert_eq!(
            store.state().library().unwrap().tab,
            Tab::Albums,
            "turning scenes must not switch the tab underneath"
        );

        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.settle();
        assert!(!store.state().scene);
        assert_eq!(store.state().scene_turn, 0, "closing resets the turn");
    }

    /// Transport still works under the scene, and `?` still helps.
    #[tokio::test]
    async fn the_scene_keeps_transport_and_help() {
        let mut store = signed_in_store();
        store.send(Action::Event(ctrl(KeyCode::Char('v'))), |_| {});
        store.settle();

        store.send(Action::Event(key(KeyCode::Char('-'))), |_| {});
        store.settle();
        assert!(store.state().player.volume < 1.0, "volume went down");

        store.send(Action::Event(key(KeyCode::Char('?'))), |_| {});
        store.settle();
        assert!(store.state().help);
    }

    /// The search field owns the keyboard on arrival, so single-key transport
    /// shortcuts must be treated as text rather than commands.
    #[tokio::test]
    async fn letters_type_into_search_instead_of_triggering_shortcuts() {
        let mut store = signed_in_store();
        assert!(
            store.state().search_focused(),
            "search should start focused"
        );

        for c in ['q', 'n', 'p', ' '] {
            store.send(Action::Event(key(KeyCode::Char(c))), |_| {});
            store.settle();
        }

        assert_eq!(store.state().library().unwrap().albums.query, "qnp ");
        assert!(
            !store.state().player.is_playing,
            "space must not have played"
        );
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
        assert!(
            store.state().picker.is_some(),
            "Ctrl-O should open the picker"
        );
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "and must not have been typed into the query"
        );

        store.send(Action::Event(key(KeyCode::Esc)), |s| s.picker = None);
        assert!(store.state().picker.is_none(), "Esc should close it");
    }

    fn queued_store(n: usize, current: usize) -> TestStore<Feature> {
        let queue: Vec<crate::subsonic::Track> = (0..n)
            .map(|i| crate::subsonic::Track {
                id: i.to_string(),
                title: format!("Track {i}"),
                ..Default::default()
            })
            .collect();
        TestStore::new(
            feature(),
            State {
                session: Session::SignedIn(Box::new(Library {
                    tab: Tab::Queue,
                    ..Default::default()
                })),
                player: player::State {
                    queue,
                    current: Some(current),
                    ..player::State::new()
                },
                ..Default::default()
            },
        )
    }

    fn cursor(store: &TestStore<Feature>) -> usize {
        store.state().library().expect("signed in").queue_cursor
    }

    #[tokio::test]
    async fn arriving_at_the_queue_lands_on_the_sounding_track() {
        let mut store = queued_store(4, 2);
        // Start elsewhere so `Tab` has to arrive at the queue.
        store.send(Action::Event(key(KeyCode::Tab)), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.tab = Tab::Albums;
            }
        });
        store.settle();

        for expected in [Tab::Artists, Tab::AllMusic, Tab::Radios, Tab::Queue] {
            store.send(Action::Event(key(KeyCode::Tab)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.tab = expected;
                    if expected == Tab::Queue {
                        l.queue_cursor = 2;
                    }
                }
            });
            store.settle();
        }

        assert_eq!(cursor(&store), 2, "should land on whatever is playing");
    }

    /// The queue tab has no search field, so bare letters are commands here.
    #[tokio::test]
    async fn the_queue_tab_moves_and_stops_at_the_end() {
        let mut store = queued_store(4, 1);
        assert_eq!(cursor(&store), 0);

        for expected in [1, 2, 3, 3, 3] {
            store.send(Action::Event(key(KeyCode::Char('j'))), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.queue_cursor = expected;
                }
            });
            store.settle();
        }
        assert_eq!(cursor(&store), 3, "must not run past the end");

        store.send(Action::Event(key(KeyCode::Up)), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.queue_cursor = 2;
            }
        });
        store.settle();
        assert_eq!(cursor(&store), 2);
    }

    /// The cursor has to stay on a real row after the queue shortens under it.
    #[tokio::test]
    async fn removing_the_last_row_pulls_the_cursor_back() {
        let mut store = queued_store(3, 1);
        for expected in [1, 2] {
            store.send(Action::Event(key(KeyCode::Down)), |s| {
                if let Session::SignedIn(l) = &mut s.session {
                    l.queue_cursor = expected;
                }
            });
            store.settle();
        }

        store.send(Action::Event(key(KeyCode::Char('d'))), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.queue_cursor = 1;
            }
        });
        store.settle();

        assert_eq!(store.state().player.queue.len(), 2, "the row is gone");
        assert_eq!(cursor(&store), 1, "cursor must not point past the end");
    }

    #[tokio::test]
    async fn emptying_the_queue_leaves_the_tab_usable() {
        let mut store = queued_store(1, 0);

        store.send(Action::Event(key(KeyCode::Char('d'))), |_| {});
        store.settle();

        assert!(store.state().player.queue.is_empty());
        assert_eq!(store.state().player.current, None, "playback should stop");
        // Nothing to remove and nowhere to move: further keys must not panic.
        for code in [KeyCode::Char('d'), KeyCode::Down, KeyCode::Enter] {
            store.send(Action::Event(key(code)), |_| {});
            store.settle();
        }
        assert_eq!(cursor(&store), 0);
    }

    /// Enter plays out of order without leaving the tab.
    #[tokio::test]
    async fn enter_plays_the_selected_entry() {
        let mut store = queued_store(3, 0);
        store.send(Action::Event(key(KeyCode::Down)), |s| {
            if let Session::SignedIn(l) = &mut s.session {
                l.queue_cursor = 1;
            }
        });
        store.settle();

        store.send(Action::Event(key(KeyCode::Enter)), |_| {});
        store.settle();

        assert_eq!(store.state().player.current, Some(1));
        assert_eq!(
            store.state().library().unwrap().tab,
            Tab::Queue,
            "playing should not navigate away"
        );
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
        assert!(
            !store.state().search_focused(),
            "Down should enter the list"
        );

        store.send(Action::Event(key(KeyCode::Up)), |_| {});
        store.settle();
        assert!(
            store.state().search_focused(),
            "Up should return to the field"
        );
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
    async fn window_focus_drives_the_accent() {
        let mut store = signed_in_store();
        assert!(
            store.state().focused,
            "a terminal that never reports is bright"
        );

        store.send(Action::Event(Event::FocusLost), |s| s.focused = false);
        store.send(Action::Event(Event::FocusGained), |s| s.focused = true);
        store.finish();
    }

    /// The focus events must not fall through to the tab as text.
    #[tokio::test]
    async fn focus_events_are_swallowed() {
        let mut store = signed_in_store();
        store.send(Action::Event(Event::FocusLost), |s| s.focused = false);
        store.settle();
        assert_eq!(
            store.state().library().unwrap().albums.query,
            "",
            "a focus change is not a keystroke"
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
            probe: Arc::new(crate::probe::fake::FakeProbe::failing(
                "no network in tests",
            )),
            directory: Arc::new(crate::directory::fake::FakeDirectory::default()),
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
        async fn artists(&self) -> anyhow::Result<Vec<crate::subsonic::Artist>> {
            anyhow::bail!("not signed in")
        }
        async fn artist_albums(&self, _: &str) -> anyhow::Result<Vec<crate::subsonic::Album>> {
            anyhow::bail!("not signed in")
        }
        async fn radios(&self) -> anyhow::Result<Vec<crate::subsonic::Station>> {
            anyhow::bail!("not signed in")
        }
        async fn create_radio(&self, _: &str, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("not signed in")
        }
        async fn delete_radio(&self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("not signed in")
        }
        async fn all_tracks(&self, _: u32, _: u32) -> anyhow::Result<Vec<crate::subsonic::Track>> {
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

    /// How many ticks the real store reduces over ten tick periods, and the
    /// most a single ticker could have made in the time that actually took.
    ///
    /// Takes the store, because it has to go before the test ends: the audio
    /// thread sleeps on the test's runtime, and panics on its way out if the
    /// runtime leaves first. With the store gone it sees its engine dropped
    /// and leaves by itself.
    async fn ticks_over_ten_periods(store: tca::Store<State, Action>) -> (u64, u64) {
        // Whatever was just sent is reduced first, so only ticks are counted.
        tokio::time::sleep(TICK * 2).await;

        // The generation counts every action the store reduces, and from
        // here on the only actions are ticks.
        let reduced = store.observe();
        let before = *reduced.borrow();
        let started = std::time::Instant::now();
        tokio::time::sleep(TICK * 10).await;
        let ticks = *reduced.borrow() - before;
        let periods = started.elapsed().as_millis() / TICK.as_millis();

        drop(store);
        tokio::time::sleep(TICK).await;

        (ticks, periods as u64 + 2)
    }

    fn signed_in() -> State {
        State {
            session: Session::SignedIn(Box::default()),
            ..Default::default()
        }
    }

    fn playback_active(active: bool) -> Action {
        Action::Player(player::Action::Delegated(
            player::Delegated::PlaybackActive(active),
        ))
    }

    /// The tick is the frame rate, so there must only ever be one. Asked for
    /// again while it runs, it must not gain a second ticker beside the
    /// first: every extra one is another twenty redraws a second.
    #[tokio::test]
    async fn asking_for_the_tick_while_it_runs_does_not_add_another() {
        let store = tca::Store::new(feature(), signed_in());
        store.send(playback_active(true));
        store.send(playback_active(true));
        store.send(playback_active(true));

        let (ticks, most_from_one) = ticks_over_ten_periods(store).await;

        assert!(
            ticks >= 5,
            "the tick is not running: {ticks} in ten periods"
        );
        assert!(
            ticks <= most_from_one,
            "{ticks} ticks, where one ticker makes at most {most_from_one}"
        );
    }

    /// The way it happens in use: the scene is opened and closed while a
    /// song plays. Each toggle asks for the tick again, and none of them
    /// stops it, because the song still needs it.
    #[tokio::test]
    async fn toggling_the_scene_during_a_song_keeps_one_tick() {
        let mut playing = signed_in();
        playing.player.is_playing = true;
        let store = tca::Store::new(feature(), playing);
        store.send(playback_active(true));
        for _ in 0..4 {
            store.send(Action::ToggleScene);
        }

        let (ticks, most_from_one) = ticks_over_ten_periods(store).await;

        assert!(
            ticks >= 5,
            "the tick is not running: {ticks} in ten periods"
        );
        assert!(
            ticks <= most_from_one,
            "{ticks} ticks, where one ticker makes at most {most_from_one}"
        );
    }

    /// And it still stops when nothing needs it.
    #[tokio::test]
    async fn the_tick_stops_when_playback_does() {
        let store = tca::Store::new(feature(), signed_in());
        store.send(playback_active(true));
        store.send(playback_active(true));
        store.send(playback_active(false));

        let (ticks, _) = ticks_over_ten_periods(store).await;

        assert_eq!(ticks, 0, "ticking with nothing playing and no scene");
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
