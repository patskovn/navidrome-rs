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
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use stream_download::http::HttpStream;
use stream_download::storage::bounded::BoundedStorageProvider;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};
use tokio::sync::mpsc;

use super::cache::{DownloadProgress, TrackCache};
use super::tap::{SampleTap, Tapped};
use crate::subsonic::Track;

/// How often the engine reports playback position while a track runs.
const POSITION_INTERVAL: Duration = Duration::from_millis(200);

/// Smallest change in download progress worth an event.
///
/// The bar is a few dozen cells wide, so anything finer than this cannot move
/// it — and every event costs a full repaint.
const BUFFER_REPORT_STEP: f64 = 0.01;

/// Memory kept behind the playhead of a live stream.
///
/// A station never ends, so its bytes go round a fixed ring rather than into
/// the cache, where a file would grow for as long as anyone listened. Two
/// megabytes is a couple of minutes of a 128k stream: ample slack for the
/// decoder, and a fixed cost however long the radio stays on.
const LIVE_BUFFER: usize = 2 * 1024 * 1024;

/// How much of a live stream to hold before starting to play.
///
/// The library default is 256 KiB, which a 128k station takes sixteen seconds
/// to deliver — sixteen seconds of silence after pressing Enter. This is about
/// three seconds at that rate, and enough for the decoder to find its frames.
const LIVE_PREFETCH: u64 = 48 * 1024;

/// Instructions to the playback thread.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "Seek is wired through to rodio but not yet bound to a key"
)]
pub enum Command {
    /// Replace whatever is playing with this track.
    Play {
        track: Track,
        url: String,
    },
    TogglePause,
    Stop,
    SetVolume(f32),
    Seek(Duration),
    /// Move playback to a named output, or back to the system default.
    SetDevice(Option<String>),
    /// Re-enumerate outputs and report them.
    RefreshDevices,
    Shutdown,
}

/// Things the playback thread reports back. The app maps these to actions.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Audio is flowing for this track.
    Started {
        track_id: String,
    },
    /// Playback position advanced.
    Position {
        elapsed: Duration,
    },
    /// How much of the current track is on disk, 0.0..=1.0.
    ///
    /// Reported only when the server gave a content length; without one there
    /// is nothing to be a fraction of.
    Buffered {
        ratio: f64,
    },
    /// The current track ran to completion.
    Finished,
    Paused(bool),
    /// Something went wrong; the message is user-facing.
    Failed(String),
    /// Available outputs as `(id, description)`, and which id is in use.
    Devices {
        names: Vec<(String, String)>,
        active: Option<String>,
    },
    /// The output vanished and playback was moved elsewhere.
    DeviceLost {
        recovered: bool,
    },
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

/// Every output the host offers, as `(id, description)`.
///
/// Identity is the device *id*, which rodio documents as stable across reboots
/// and reconnections; the description is only ever shown. Matching on the
/// human-readable name would break the moment two devices shared one.
pub fn output_devices() -> Vec<(String, String)> {
    let host = rodio::cpal::default_host();
    host.output_devices()
        .map(|devices| {
            devices
                .map(|d| {
                    let id = d.id().map(|id| format!("{id:?}")).unwrap_or_default();
                    let label = d
                        .description()
                        .map(|desc| desc.name().to_string())
                        .unwrap_or_else(|_| "Unknown output".to_string());
                    (id, label)
                })
                .filter(|(id, _)| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Opens a sink on `name`, or the default when absent or not found.
///
/// The error callback is the whole point: cpal reports `DeviceNotAvailable`
/// here when a device is unplugged, and without it playback simply stops with
/// nothing noticing.
fn open_sink(
    name: Option<&str>,
    lost: tokio::sync::mpsc::UnboundedSender<()>,
) -> Result<(MixerDeviceSink, Option<String>), String> {
    let on_error = move |err: rodio::cpal::StreamError| {
        log::warn!("audio stream error: {err}");
        let _ = lost.send(());
    };

    let chosen = name.and_then(|wanted| {
        rodio::cpal::default_host()
            .output_devices()
            .ok()?
            .find(|d| d.id().is_ok_and(|id| format!("{id:?}") == wanted))
    });

    let builder = match chosen {
        Some(device) => DeviceSinkBuilder::from_device(device),
        None => DeviceSinkBuilder::from_default_device(),
    }
    .map_err(|err| format!("{err}"))?;

    let active = name.map(str::to_string);
    let mut sink = builder
        .with_error_callback(on_error)
        .open_stream()
        .map_err(|err| format!("{err}"))?;

    // Every rebuild drops the previous sink on purpose, and rodio warns about
    // that on stderr — which lands on top of the alternate screen and corrupts
    // the TUI. We manage the lifecycle deliberately, so the warning is noise.
    sink.log_on_drop(false);

    Ok((sink, active))
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
/// The `DownloadProgress` is `None` for a cache hit: the whole file is already
/// on disk, so there is nothing to watch.
async fn open_source(
    cache: &Arc<TrackCache>,
    track: &Track,
    url: &str,
) -> anyhow::Result<(Playable, Option<DownloadProgress>)> {
    if let Some(live) = &track.live_url {
        log::debug!("tuning in to {live}");
        let provider = BoundedStorageProvider::new(
            MemoryStorageProvider,
            NonZeroUsize::new(LIVE_BUFFER).expect("a nonzero constant"),
        );
        // Our own client, not the library's default: without a User-Agent
        // some stations answer 403 to the very URL the probe just accepted.
        let client = reqwest::Client::builder()
            .user_agent(crate::probe::USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        let http = HttpStream::new(client, live.parse()?)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let settings = Settings::default().prefetch_bytes(LIVE_PREFETCH);
        let stream = StreamDownload::from_stream(http, provider, settings)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        // Nothing to watch: there is no total for progress to be a part of.
        return Ok((Box::new(stream), None));
    }

    if let Some(path) = cache.lookup(&track.id, track.suffix.as_deref()) {
        log::debug!("cache hit for {}", track.id);
        return Ok((Box::new(std::fs::File::open(path)?), None));
    }

    log::debug!("streaming {} from the server", track.id);
    let provider = cache.storage_provider(&track.id, track.suffix.as_deref());
    // Taken before the provider is consumed by `stream-download`.
    let progress = provider.progress();
    let stream = StreamDownload::new_http(url.parse()?, provider, Settings::default())
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok((Box::new(stream), Some(progress)))
}

/// A decoder for `source`.
///
/// A live stream has no end, so it must not be treated as seekable: the
/// decoder would otherwise try to find the end to work out a duration, and
/// there is none to find.
fn decode(
    source: Playable,
    live: bool,
) -> Result<rodio::Decoder<Playable>, rodio::decoder::DecoderError> {
    if live {
        rodio::Decoder::builder()
            .with_data(source)
            .with_seekable(false)
            .with_gapless(false)
            .build()
    } else {
        rodio::Decoder::new(source)
    }
}

fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
    tap: SampleTap,
    cache: Arc<TrackCache>,
    runtime: tokio::runtime::Handle,
) {
    // Device loss arrives on its own channel, from cpal's callback thread.
    let (lost_tx, mut lost_rx) = mpsc::unbounded_channel::<()>();

    let mut device_name: Option<String> = None;
    let (mut device, _) = match open_sink(None, lost_tx.clone()) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = events.send(Event::Failed(format!("no audio output: {err}")));
            return;
        }
    };
    let mut player = Player::connect_new(device.mixer());

    // Enough to put playback back after a rebuild: a fresh sink means a fresh
    // `Player`, so the track has to be re-opened and seeked.
    let mut current: Option<(Track, String)> = None;
    let mut volume = 1.0f32;

    // The download behind the current track, while one is in flight. `None`
    // means nothing to watch: a cache hit, or nothing playing.
    let mut buffering: Option<DownloadProgress> = None;
    // Last value reported, so an unchanging one is not re-sent. Every event
    // wakes the store and costs a redraw, and a stalled download would
    // otherwise repaint five times a second saying the same thing.
    let mut reported_buffer = f64::NEG_INFINITY;

    // Tracks whether we have already reported the end of the current track, so
    // an idle player does not emit `Finished` on every tick.
    let mut playing = false;

    let _ = events.send(Event::Devices {
        names: output_devices(),
        active: device_name.clone(),
    });

    loop {
        // `recv_timeout` equivalent: wait for a command, but wake regularly to
        // report position and notice the track ending.
        // Wake for a command, for device loss, or on the position tick.
        let mut device_lost = false;
        let command = runtime.block_on(async {
            tokio::select! {
                command = commands.recv() => Ok(command),
                _ = lost_rx.recv() => {
                    device_lost = true;
                    Ok(None)
                }
                _ = tokio::time::sleep(POSITION_INTERVAL) => Err(()),
            }
        });

        if device_lost {
            // Drain any pile-up: one unplug can fire the callback repeatedly.
            while lost_rx.try_recv().is_ok() {}
            log::warn!("output device lost; rebuilding");

            let resume_at = player.get_pos();
            let was_playing = playing && !player.is_paused();

            match open_sink(device_name.as_deref(), lost_tx.clone())
                .or_else(|_| open_sink(None, lost_tx.clone()))
            {
                Ok((new_device, active)) => {
                    device = new_device;
                    device_name = active;
                    player = Player::connect_new(device.mixer());
                    // Volume lives on the `Player`, so a rebuild resets it.
                    player.set_volume(volume);

                    if was_playing
                        && let Some((track, url)) = current.clone()
                        && let Ok((source, progress)) =
                            runtime.block_on(open_source(&cache, &track, &url))
                        && let Ok(decoded) = decode(source, track.live_url.is_some())
                    {
                        player.append(Tapped::new(decoded, tap.clone()));
                        player.play();
                        // A rebuild re-opens the track, so this is a new
                        // download to watch — or a cache hit, if it finished.
                        buffering = progress;
                        if let Err(err) = player.try_seek(resume_at) {
                            log::warn!("could not resume at {resume_at:?}: {err}");
                        }
                    } else {
                        playing = false;
                    }
                    let _ = events.send(Event::DeviceLost { recovered: true });
                    let _ = events.send(Event::Devices {
                        names: output_devices(),
                        active: device_name.clone(),
                    });
                }
                Err(err) => {
                    playing = false;
                    let _ = events.send(Event::Failed(format!("lost audio output: {err}")));
                    let _ = events.send(Event::DeviceLost { recovered: false });
                }
            }
            continue;
        }

        match command {
            Ok(Some(Command::Shutdown)) | Ok(None) => break,
            Ok(Some(Command::Play { track, url })) => {
                player.clear();
                tap.mark_idle();
                current = Some((track.clone(), url.clone()));

                match runtime.block_on(open_source(&cache, &track, &url)) {
                    Ok((source, progress)) => match decode(source, track.live_url.is_some()) {
                        Ok(decoded) => {
                            player.append(Tapped::new(decoded, tap.clone()));
                            player.play();
                            playing = true;
                            let _ = events.send(Event::Started {
                                track_id: track.id.clone(),
                            });
                            let _ = events.send(Event::Paused(false));
                            // Reported up front so the bar is right on the very
                            // first frame: a cache hit is wholly buffered, and
                            // a fresh stream starts near zero. A live stream
                            // has no whole to be a fraction of, so says nothing.
                            if track.live_url.is_none() {
                                let _ = events.send(Event::Buffered {
                                    ratio: match &progress {
                                        Some(p) => p.fraction().unwrap_or(0.0),
                                        None => 1.0,
                                    },
                                });
                            }
                            buffering = progress;
                            reported_buffer = f64::NEG_INFINITY;
                        }
                        Err(err) => {
                            let _ = events.send(Event::Failed(format!(
                                "cannot decode {}: {err}",
                                track.title
                            )));
                        }
                    },
                    Err(err) => {
                        let _ = events.send(Event::Failed(format!(
                            "cannot play {}: {err:#}",
                            track.title
                        )));
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
                current = None;
                buffering = None;
                reported_buffer = f64::NEG_INFINITY;
                let _ = events.send(Event::Paused(true));
            }
            Ok(Some(Command::SetVolume(level))) => {
                volume = level.clamp(0.0, 2.0);
                player.set_volume(volume);
            }

            Ok(Some(Command::RefreshDevices)) => {
                let _ = events.send(Event::Devices {
                    names: output_devices(),
                    active: device_name.clone(),
                });
            }

            Ok(Some(Command::SetDevice(name))) => {
                let resume_at = player.get_pos();
                let was_playing = playing && !player.is_paused();

                match open_sink(name.as_deref(), lost_tx.clone()) {
                    Ok((new_device, active)) => {
                        device = new_device;
                        device_name = active;
                        player = Player::connect_new(device.mixer());
                        player.set_volume(volume);

                        if was_playing
                            && let Some((track, url)) = current.clone()
                            && let Ok((source, progress)) =
                                runtime.block_on(open_source(&cache, &track, &url))
                            && let Ok(decoded) = decode(source, track.live_url.is_some())
                        {
                            player.append(Tapped::new(decoded, tap.clone()));
                            player.play();
                            buffering = progress;
                            let _ = player.try_seek(resume_at);
                        } else {
                            playing = false;
                        }
                        let _ = events.send(Event::Devices {
                            names: output_devices(),
                            active: device_name.clone(),
                        });
                    }
                    Err(err) => {
                        let _ =
                            events.send(Event::Failed(format!("cannot use that output: {err}")));
                    }
                }
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
                buffering = None;
                let _ = events.send(Event::Finished);
            } else if !player.is_paused() {
                let _ = events.send(Event::Position {
                    elapsed: player.get_pos(),
                });
            }
        }

        // Reported even while paused: the download keeps running, and watching
        // it fill is half the point of showing it.
        if let Some(progress) = &buffering
            && let Some(ratio) = progress.fraction()
        {
            // A move too small to shift a cell on the bar is not worth a frame.
            if ratio - reported_buffer >= BUFFER_REPORT_STEP || ratio >= 1.0 {
                reported_buffer = ratio;
                let _ = events.send(Event::Buffered { ratio });
            }
            if ratio >= 1.0 {
                // Nothing left to watch; stop waking the store for it.
                buffering = None;
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

#[cfg(test)]
mod device_probe {
    /// `cargo test list_outputs -- --nocapture --ignored`
    #[test]
    #[ignore = "depends on the host's audio hardware"]
    fn list_outputs() {
        for (id, label) in super::output_devices() {
            println!("PROBE {label}  [{id}]");
        }
    }
}

#[cfg(test)]
mod live_probe {
    /// Tunes in to a real station and decodes a few seconds, no device needed.
    ///
    /// Opt-in, because it reaches a stranger's server:
    /// `NAVIDROME_RS_LIVE_URL=<stream> cargo test tune_in -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "reaches a real radio station; set NAVIDROME_RS_LIVE_URL"]
    async fn tune_in() {
        use rodio::Source;
        use std::sync::Arc;

        let url = std::env::var("NAVIDROME_RS_LIVE_URL").expect("set NAVIDROME_RS_LIVE_URL");
        let dir = std::env::temp_dir().join(format!("navidrome-live-{}", std::process::id()));
        let cache = Arc::new(super::TrackCache::new(&dir, 1024).unwrap());
        let track = crate::subsonic::Track {
            id: "radio:probe".into(),
            title: "probe".into(),
            live_url: Some(url.clone()),
            ..Default::default()
        };

        let started = std::time::Instant::now();
        let (source, progress) = super::open_source(&cache, &track, "unused").await.unwrap();
        assert!(progress.is_none(), "a live stream has nothing to measure");
        let connected = started.elapsed();

        // Decoding blocks on the network; keep it off the async workers.
        let report = tokio::task::spawn_blocking(move || {
            let decoder = super::decode(source, true).expect("decodable");
            let (rate, channels) = (decoder.sample_rate(), decoder.channels());
            let wanted = rate.get() as usize * channels.get() as usize * 3;
            let samples: Vec<f32> = decoder.take(wanted).collect();
            let audible = samples.iter().filter(|s| s.abs() > 1e-4).count();
            (rate, channels, samples.len(), audible, wanted)
        })
        .await
        .unwrap();

        let (rate, channels, got, audible, wanted) = report;
        println!(
            "LIVE {url}: connected in {connected:?}, {rate:?} Hz × {channels:?}, \
             decoded {got}/{wanted} samples ({audible} audible) in {:?}",
            started.elapsed()
        );
        assert_eq!(got, wanted, "the stream ended early");
        assert!(audible > got / 10, "that was mostly silence");
        let _ = std::fs::remove_dir_all(dir);
    }
}
