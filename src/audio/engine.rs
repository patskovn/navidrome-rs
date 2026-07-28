//! Playback, isolated from the reducer.
//!
//! Two constraints shape this module:
//!
//! * `cpal`'s stream handle is not `Send` on every platform, so the device must
//!   live on one dedicated OS thread rather than a tokio task.
//! * Reducer `State` must stay `Send + Sync` and cheap to compare, so the
//!   `Player` cannot live there. It is reached only through commands.
//!
//! The result is a thread that owns the device, a command channel in, and an
//! event channel out that the app turns back into actions.

use std::io::{Read, Seek};
use std::sync::Arc;
use std::time::Duration;

use rodio::{DeviceSinkBuilder, Player};
use stream_download::{Settings, StreamDownload};
use tokio::sync::mpsc;

use super::cache::TrackCache;
use super::tap::{SampleTap, Tapped};
use crate::subsonic::Track;

/// How often the engine reports playback position while a track runs.
const POSITION_INTERVAL: Duration = Duration::from_millis(200);

/// Instructions to the playback thread.
#[derive(Debug)]
#[allow(dead_code, reason = "Seek is wired through to rodio but not yet bound to a key")]
pub enum Command {
    /// Replace whatever is playing with this track.
    Play { track: Track, url: String },
    TogglePause,
    Stop,
    SetVolume(f32),
    Seek(Duration),
    Shutdown,
}

/// Things the playback thread reports back. The app maps these to actions.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Audio is flowing for this track.
    Started { track_id: String },
    /// Playback position advanced.
    Position { elapsed: Duration },
    /// The current track ran to completion.
    Finished,
    Paused(bool),
    /// Something went wrong; the message is user-facing.
    Failed(String),
}

/// Handle used by the rest of the app. Cheap to clone.
#[derive(Debug, Clone)]
pub struct AudioEngine {
    commands: mpsc::UnboundedSender<Command>,
    tap: SampleTap,
}

impl AudioEngine {
    /// Starts the playback thread. The returned receiver yields [`Event`]s.
    pub fn start(cache: Arc<TrackCache>) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let tap = SampleTap::new();

        let thread_tap = tap.clone();
        let runtime = tokio::runtime::Handle::current();
        std::thread::Builder::new()
            .name("audio".to_string())
            .spawn(move || {
                run(command_rx, event_tx, thread_tap, cache, runtime);
            })
            .expect("spawning the audio thread");

        (
            Self {
                commands: command_tx,
                tap,
            },
            event_rx,
        )
    }

    /// Samples for the visualiser. Read directly by the UI, never via state.
    pub fn tap(&self) -> &SampleTap {
        &self.tap
    }

    pub fn send(&self, command: Command) {
        // A dead audio thread should degrade to silence, not take down the UI.
        if self.commands.send(command).is_err() {
            log::error!("audio thread is gone; command dropped");
        }
    }
}

/// Either a fully cached file or a progressive HTTP stream, both of which are
/// `Read + Seek` and so decodable.
type Playable = Box<dyn ReadSeekSend>;

pub trait ReadSeekSend: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeekSend for T {}

/// Resolves a track to something decodable, preferring the cache.
///
/// On a miss it streams over HTTP *into* the cache, so playback can start
/// before the download finishes and the file is still cached at the end.
async fn open_source(
    cache: &Arc<TrackCache>,
    track: &Track,
    url: &str,
) -> anyhow::Result<Playable> {
    if let Some(path) = cache.lookup(&track.id, track.suffix.as_deref()) {
        log::debug!("cache hit for {}", track.id);
        return Ok(Box::new(std::fs::File::open(path)?));
    }

    log::debug!("streaming {} from the server", track.id);
    let provider = cache.storage_provider(&track.id, track.suffix.as_deref());
    let stream = StreamDownload::new_http(
        url.parse()?,
        provider,
        Settings::default(),
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(Box::new(stream))
}

fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
    tap: SampleTap,
    cache: Arc<TrackCache>,
    runtime: tokio::runtime::Handle,
) {
    let device = match DeviceSinkBuilder::open_default_sink() {
        Ok(device) => device,
        Err(err) => {
            let _ = events.send(Event::Failed(format!("no audio output: {err}")));
            return;
        }
    };
    let player = Player::connect_new(device.mixer());

    // Tracks whether we have already reported the end of the current track, so
    // an idle player does not emit `Finished` on every tick.
    let mut playing = false;

    loop {
        // `recv_timeout` equivalent: wait for a command, but wake regularly to
        // report position and notice the track ending.
        let command = runtime.block_on(async {
            tokio::time::timeout(POSITION_INTERVAL, commands.recv()).await
        });

        match command {
            Ok(Some(Command::Shutdown)) | Ok(None) => break,
            Ok(Some(Command::Play { track, url })) => {
                player.clear();
                tap.mark_idle();

                match runtime.block_on(open_source(&cache, &track, &url)) {
                    Ok(source) => match rodio::Decoder::new(source) {
                        Ok(decoded) => {
                            player.append(Tapped::new(decoded, tap.clone()));
                            player.play();
                            playing = true;
                            let _ = events.send(Event::Started {
                                track_id: track.id.clone(),
                            });
                            let _ = events.send(Event::Paused(false));
                        }
                        Err(err) => {
                            let _ = events
                                .send(Event::Failed(format!("cannot decode {}: {err}", track.title)));
                        }
                    },
                    Err(err) => {
                        let _ = events
                            .send(Event::Failed(format!("cannot play {}: {err:#}", track.title)));
                    }
                }
            }
            Ok(Some(Command::TogglePause)) => {
                if player.is_paused() {
                    player.play();
                } else {
                    player.pause();
                }
                let _ = events.send(Event::Paused(player.is_paused()));
            }
            Ok(Some(Command::Stop)) => {
                player.clear();
                tap.mark_idle();
                playing = false;
                let _ = events.send(Event::Paused(true));
            }
            Ok(Some(Command::SetVolume(volume))) => {
                player.set_volume(volume.clamp(0.0, 2.0));
            }
            Ok(Some(Command::Seek(pos))) => {
                if let Err(err) = player.try_seek(pos) {
                    log::warn!("seek failed: {err}");
                }
            }
            // Timed out: no command, just do the periodic work.
            Err(_) => {}
        }

        if playing {
            if player.empty() {
                playing = false;
                tap.mark_idle();
                let _ = events.send(Event::Finished);
            } else if !player.is_paused() {
                let _ = events.send(Event::Position {
                    elapsed: player.get_pos(),
                });
            }
        }
    }

    log::debug!("audio thread shutting down");
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn events_compare_by_value() {
        // The app diffs these into state, so equality has to be structural.
        assert_eq!(
            Event::Started {
                track_id: "a".into()
            },
            Event::Started {
                track_id: "a".into()
            }
        );
        assert_ne!(Event::Paused(true), Event::Paused(false));
    }

    #[tokio::test]
    async fn commands_to_a_dead_engine_do_not_panic() {
        let (tx, rx) = mpsc::unbounded_channel();
        let engine = AudioEngine {
            commands: tx,
            tap: SampleTap::new(),
        };
        drop(rx);
        // Losing the audio thread must degrade to silence, not abort the UI.
        engine.send(Command::TogglePause);
    }
}
