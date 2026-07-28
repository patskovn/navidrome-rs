//! Transport: the queue, what is playing, and the controls.
//!
//! Note what is *not* here: the `Player`, the device, the sample buffer. This
//! feature holds only plain data, so it stays `Send + Sync`, cheap to compare,
//! and drivable by `tca::TestStore`.

use std::time::Duration;

use tca::Effect;
use tca::Reducer;

use crate::audio::AudioEngine;
use crate::audio::engine::Command;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;

/// Repeat/shuffle are deliberately absent for now; the queue is linear.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub queue: Vec<Track>,
    /// Index into `queue`, or `None` when nothing has been started.
    pub current: Option<usize>,
    pub is_playing: bool,
    pub elapsed: Duration,
    /// 0.0..=2.0, where 1.0 is unity gain.
    pub volume: f32,
    pub error: Option<String>,
}

impl State {
    pub fn new() -> Self {
        Self {
            volume: 1.0,
            ..Default::default()
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|i| self.queue.get(i))
    }

    /// Fraction of the current track elapsed, for the progress bar.
    pub fn progress(&self) -> f64 {
        match self.current_track().and_then(|t| t.duration) {
            Some(total) if total > 0 => {
                (self.elapsed.as_secs_f64() / total as f64).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }
}

#[derive(Debug)]
pub enum Action {
    /// Replace the queue and start at `index`.
    SetQueue { tracks: Vec<Track>, index: usize },
    TogglePause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    /// Reported by the audio thread.
    Engine(crate::audio::Event),
    /// Emitted internally once a track has been handed to the engine.
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    /// The visualiser tick should start or stop.
    PlaybackActive(bool),
}

/// Volume step per key press.
const VOLUME_STEP: f32 = 0.05;

pub struct Feature {
    pub library: SharedLibrary,
    pub audio: AudioEngine,
}

impl Feature {
    /// Hands the current track to the audio thread.
    fn start_current(&self, state: &mut State) -> Effect<Action> {
        let Some(track) = state.current_track().cloned() else {
            return Effect::none();
        };

        match self.library.stream_url(&track.id) {
            Ok(url) => {
                state.error = None;
                self.audio.send(Command::Play { track, url });
                Effect::send(Action::Delegated(Delegated::PlaybackActive(true)))
            }
            Err(err) => {
                state.error = Some(format!("{err:#}"));
                Effect::none()
            }
        }
    }
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        match action {
            Action::SetQueue { tracks, index } => {
                if tracks.is_empty() {
                    return Effect::none();
                }
                state.queue = tracks;
                state.current = Some(index.min(state.queue.len() - 1));
                state.elapsed = Duration::ZERO;
                self.start_current(state)
            }

            Action::TogglePause => {
                if state.current.is_none() {
                    return Effect::none();
                }
                self.audio.send(Command::TogglePause);
                Effect::none()
            }

            Action::Next => {
                let Some(current) = state.current else {
                    return Effect::none();
                };
                if current + 1 >= state.queue.len() {
                    // End of queue: stop rather than wrapping, which would be a
                    // surprising way to end an album.
                    self.audio.send(Command::Stop);
                    state.is_playing = false;
                    return Effect::send(Action::Delegated(Delegated::PlaybackActive(false)));
                }
                state.current = Some(current + 1);
                state.elapsed = Duration::ZERO;
                self.start_current(state)
            }

            Action::Previous => {
                let Some(current) = state.current else {
                    return Effect::none();
                };
                // Match every other music player: restart the track unless we
                // are already near its start.
                if state.elapsed > Duration::from_secs(3) || current == 0 {
                    state.elapsed = Duration::ZERO;
                    return self.start_current(state);
                }
                state.current = Some(current - 1);
                state.elapsed = Duration::ZERO;
                self.start_current(state)
            }

            Action::VolumeUp => {
                state.volume = (state.volume + VOLUME_STEP).min(2.0);
                self.audio.send(Command::SetVolume(state.volume));
                Effect::none()
            }

            Action::VolumeDown => {
                state.volume = (state.volume - VOLUME_STEP).max(0.0);
                self.audio.send(Command::SetVolume(state.volume));
                Effect::none()
            }

            Action::Engine(event) => self.reduce_engine_event(state, event),

            Action::Delegated(_) => Effect::none(),
        }
    }
}

impl Feature {
    fn reduce_engine_event(
        &self,
        state: &mut State,
        event: crate::audio::Event,
    ) -> Effect<Action> {
        use crate::audio::Event;
        match event {
            Event::Started { .. } => {
                state.is_playing = true;
                state.elapsed = Duration::ZERO;
                Effect::none()
            }
            Event::Position { elapsed } => {
                state.elapsed = elapsed;
                Effect::none()
            }
            Event::Paused(paused) => {
                state.is_playing = !paused;
                Effect::send(Action::Delegated(Delegated::PlaybackActive(!paused)))
            }
            Event::Finished => {
                // Auto-advance is what makes an album play through.
                Effect::send(Action::Next)
            }
            Event::Failed(message) => {
                state.error = Some(message);
                state.is_playing = false;
                Effect::send(Action::Delegated(Delegated::PlaybackActive(false)))
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::audio::TrackCache;
    use crate::subsonic::fake::FakeLibrary;
    use std::sync::Arc;
    use tca::TestStore;

    fn track(id: &str) -> Track {
        Track {
            id: id.into(),
            title: format!("Track {id}"),
            duration: Some(100),
            ..Default::default()
        }
    }

    /// Builds a feature whose audio engine is detached: commands go nowhere,
    /// which is exactly what a reducer test wants.
    fn feature() -> Feature {
        let dir = std::env::temp_dir().join(format!("navidrome-player-test-{}", std::process::id()));
        let cache = Arc::new(TrackCache::new(&dir, 1024).unwrap());
        let (audio, _events) = AudioEngine::start(cache);
        Feature {
            library: Arc::new(FakeLibrary::default()),
            audio,
        }
    }

    fn store() -> TestStore<Feature> {
        TestStore::new(feature(), State::new())
    }

    #[tokio::test]
    async fn setting_a_queue_starts_the_first_track() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![track("a"), track("b")],
                index: 0,
            },
            |s| {
                s.queue = vec![track("a"), track("b")];
                s.current = Some(0);
            },
        );
        // Starting playback asks the visualiser tick to begin.
        store.settle();
        store.finish();
    }

    #[tokio::test]
    async fn an_empty_queue_is_ignored() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![],
                index: 0,
            },
            |_| {},
        );
        assert_eq!(store.state().current, None);
        store.finish();
    }

    #[tokio::test]
    async fn finishing_a_track_advances_the_queue() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![track("a"), track("b")],
                index: 0,
            },
            |s| {
                s.queue = vec![track("a"), track("b")];
                s.current = Some(0);
            },
        );
        store.settle();

        store.send(Action::Engine(crate::audio::Event::Finished), |_| {});
        store.settle();
        assert_eq!(store.state().current, Some(1), "should advance to track b");
        store.finish();
    }

    #[tokio::test]
    async fn finishing_the_last_track_stops_rather_than_wrapping() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![track("only")],
                index: 0,
            },
            |s| {
                s.queue = vec![track("only")];
                s.current = Some(0);
            },
        );
        store.settle();

        store.send(Action::Engine(crate::audio::Event::Finished), |_| {});
        store.settle();

        assert_eq!(store.state().current, Some(0), "queue must not wrap around");
        assert!(!store.state().is_playing);
        store.finish();
    }

    #[tokio::test]
    async fn previous_restarts_the_track_when_past_the_grace_period() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![track("a"), track("b")],
                index: 1,
            },
            |s| {
                s.queue = vec![track("a"), track("b")];
                s.current = Some(1);
            },
        );
        store.settle();

        store.send(
            Action::Engine(crate::audio::Event::Position {
                elapsed: Duration::from_secs(10),
            }),
            |s| s.elapsed = Duration::from_secs(10),
        );

        store.send(Action::Previous, |s| s.elapsed = Duration::ZERO);
        store.settle();
        assert_eq!(
            store.state().current,
            Some(1),
            "10s in, previous should restart rather than skip back"
        );
        store.finish();
    }

    #[tokio::test]
    async fn previous_skips_back_near_the_start_of_a_track() {
        let mut store = store();
        store.send(
            Action::SetQueue {
                tracks: vec![track("a"), track("b")],
                index: 1,
            },
            |s| {
                s.queue = vec![track("a"), track("b")];
                s.current = Some(1);
            },
        );
        store.settle();

        store.send(Action::Previous, |s| s.current = Some(0));
        store.settle();
        assert_eq!(store.state().current, Some(0));
        store.finish();
    }

    #[tokio::test]
    async fn volume_is_clamped_at_both_ends() {
        let mut store = store();
        for _ in 0..60 {
            store.send(Action::VolumeUp, |s| {
                s.volume = (s.volume + VOLUME_STEP).min(2.0)
            });
        }
        assert_eq!(store.state().volume, 2.0);

        for _ in 0..80 {
            store.send(Action::VolumeDown, |s| {
                s.volume = (s.volume - VOLUME_STEP).max(0.0)
            });
        }
        assert_eq!(store.state().volume, 0.0);
        store.finish();
    }

    #[test]
    fn progress_is_zero_without_a_known_duration() {
        let mut state = State::new();
        state.queue = vec![Track {
            id: "x".into(),
            duration: None,
            ..Default::default()
        }];
        state.current = Some(0);
        state.elapsed = Duration::from_secs(30);
        assert_eq!(state.progress(), 0.0);
    }

    #[test]
    fn progress_is_a_fraction_of_the_track() {
        let mut state = State::new();
        state.queue = vec![track("a")]; // 100s long
        state.current = Some(0);
        state.elapsed = Duration::from_secs(25);
        assert!((state.progress() - 0.25).abs() < f64::EPSILON);
    }
}
