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
use std::time::{Duration, Instant};

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use stream_download::http::HttpStream;
use stream_download::storage::bounded::BoundedStorageProvider;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};
use tokio::sync::mpsc;

use super::cache::{DownloadProgress, TrackCache};
use super::relay::Relay;
use super::tap::{SampleTap, Tapped};
use crate::subsonic::Track;

/// How often the engine reports playback position while a track runs.
const POSITION_INTERVAL: Duration = Duration::from_millis(200);

/// How often to look for a better output: the system default moving (AirPods
/// put in, a monitor unplugged) or a chosen device coming back.
///
/// Looked for rather than waited for, because on macOS nothing tells us. cpal
/// watches only the device a stream is open on, and for the default device it
/// swallows even that: when the device dies it pauses the stream and reports
/// to a callback that does nothing, so playback just goes silent.
const OUTPUT_CHECK: Duration = Duration::from_secs(1);

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
    /// Play on a named output while it is plugged in, or always follow the
    /// system default.
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
    /// The chosen output vanished and playback moved to the default
    /// (`recovered`), or there is no output to play on at all.
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
                .filter_map(|d| {
                    let label = d
                        .description()
                        .map(|desc| desc.name().to_string())
                        .unwrap_or_else(|_| "Unknown output".to_string());
                    Some((device_id(&d)?, label))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A device's id as the app knows it: rodio's stable id, printed.
fn device_id(device: &rodio::cpal::Device) -> Option<String> {
    device
        .id()
        .ok()
        .map(|id| format!("{id:?}"))
        .filter(|id| !id.is_empty())
}

/// Where playback belongs: the chosen output while it is plugged in,
/// otherwise the system default as it is right now.
///
/// Following the default is what makes AirPods and monitors work: macOS
/// moves it to them as they come and back as they go.
fn pick_output(
    chosen: Option<&str>,
    available: &[String],
    default: Option<&str>,
) -> Option<String> {
    chosen
        .filter(|wanted| available.iter().any(|id| id == wanted))
        .or(default)
        .map(str::to_string)
}

/// [`pick_output`] against the devices as they are now.
fn wanted_output(chosen: Option<&str>) -> Option<String> {
    let host = rodio::cpal::default_host();
    // Listing every device is only needed to see whether the chosen is here.
    let available: Vec<String> = match chosen {
        Some(_) => host
            .output_devices()
            .map(|devices| devices.filter_map(|d| device_id(&d)).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let default = host.default_output_device().and_then(|d| device_id(&d));
    pick_output(chosen, &available, default.as_deref())
}

/// Opens the output `id` and plugs the relay into it.
///
/// The error callback reports `opening`, so a late complaint from a device
/// already left behind is not taken for the current one failing.
fn open_output(
    id: &str,
    relay: &Relay,
    opening: u64,
    lost: mpsc::UnboundedSender<u64>,
) -> Result<MixerDeviceSink, String> {
    let device = rodio::cpal::default_host()
        .output_devices()
        .map_err(|err| format!("{err}"))?
        .find(|d| device_id(d).as_deref() == Some(id))
        .ok_or_else(|| "the device is gone".to_string())?;

    let on_error = move |err: rodio::cpal::StreamError| {
        log::warn!("audio stream error: {err}");
        let _ = lost.send(opening);
    };
    let mut sink = DeviceSinkBuilder::from_device(device)
        .map_err(|err| format!("{err}"))?
        .with_error_callback(on_error)
        .open_stream()
        .map_err(|err| format!("{err}"))?;

    // Every move drops the previous sink on purpose, and rodio warns about
    // that on stderr, which lands on top of the alternate screen and corrupts
    // the TUI. We manage the lifecycle deliberately, so the warning is noise.
    sink.log_on_drop(false);

    sink.mixer().add(relay.outlet());
    Ok(sink)
}

/// What an output check did.
#[derive(Debug)]
enum Moved {
    /// Already where it belongs, or could not move and still plays.
    Stayed,
    /// Now on a new device. `fell_back`: not the one the user chose.
    To { fell_back: bool },
    /// The device it should be on would not open.
    Failed {
        err: String,
        /// Nothing is playing it either: no device is open at all.
        nowhere: bool,
    },
}

/// The device playback is on, and when to move it.
struct Output {
    relay: Relay,
    lost: mpsc::UnboundedSender<u64>,
    /// The output the user picked; `None` follows the system default.
    chosen: Option<String>,
    /// The open device and its id; `None` while there is nowhere to play.
    device: Option<(MixerDeviceSink, String)>,
    /// Counts devices opened, to tell the current one's errors from stale ones.
    opening: u64,
    /// The current device reported an error and must be reopened.
    broken: bool,
    checked: Instant,
    /// What the app has been told, so each is said once rather than every
    /// second it stays true.
    told_nowhere: bool,
    told_fell_back: bool,
}

impl Output {
    fn new(relay: Relay, lost: mpsc::UnboundedSender<u64>) -> Self {
        Self {
            relay,
            lost,
            chosen: None,
            device: None,
            opening: 0,
            broken: false,
            checked: Instant::now(),
            told_nowhere: false,
            told_fell_back: false,
        }
    }

    /// Time to look again: the device complained, or a while has passed.
    fn due(&self) -> bool {
        self.broken || self.checked.elapsed() >= OUTPUT_CHECK
    }

    /// Moves playback to where it belongs, if it is not there already.
    ///
    /// The new device is opened before the old one is let go, so a device
    /// that will not open costs nothing: the old one plays on.
    fn check(&mut self) -> Moved {
        self.checked = Instant::now();
        let working = self.device.is_some() && !self.broken;
        let Some(target) = wanted_output(self.chosen.as_deref()) else {
            return if working {
                Moved::Stayed
            } else {
                Moved::Failed {
                    err: "no output device".to_string(),
                    nowhere: true,
                }
            };
        };
        if working && self.device.as_ref().is_some_and(|(_, id)| *id == target) {
            return Moved::Stayed;
        }

        self.opening += 1;
        match open_output(&target, &self.relay, self.opening, self.lost.clone()) {
            Ok(sink) => {
                log::info!("audio output is now {target}");
                // The old device goes here; its outlet has already ended.
                self.device = Some((sink, target.clone()));
                self.broken = false;
                Moved::To {
                    fell_back: self.chosen.as_ref().is_some_and(|c| *c != target),
                }
            }
            Err(err) => {
                log::warn!("cannot open audio output {target}: {err}");
                Moved::Failed {
                    err,
                    nowhere: !working,
                }
            }
        }
    }

    /// A device's error callback fired. Only the current device's counts.
    fn complained(&mut self, opening: u64) {
        if opening == self.opening {
            log::warn!("audio output failed; reopening");
            self.broken = true;
        }
    }

    /// What the app needs to hear after a check. `idle` is whether the
    /// player is silent anyway: paused, or with nothing to play.
    fn events(&mut self, moved: &Moved, idle: bool) -> Vec<Event> {
        match moved {
            Moved::Stayed => Vec::new(),
            Moved::To { fell_back } => {
                let mut said = vec![Event::Devices {
                    names: output_devices(),
                    active: self.chosen.clone(),
                }];
                if *fell_back && !self.told_fell_back {
                    said.push(Event::DeviceLost { recovered: true });
                }
                self.told_fell_back = *fell_back;
                // Back from nowhere: the app marked playback stopped.
                if std::mem::take(&mut self.told_nowhere) {
                    said.push(Event::Paused(idle));
                }
                said
            }
            Moved::Failed { nowhere: true, .. } if !self.told_nowhere => {
                self.told_nowhere = true;
                vec![Event::DeviceLost { recovered: false }]
            }
            Moved::Failed { .. } => Vec::new(),
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
    // Device errors arrive on their own channel, from cpal's callback thread,
    // tagged with which opening of a device they came from.
    let (lost_tx, mut lost_rx) = mpsc::unbounded_channel::<u64>();

    // The player outlives every device: it plays into a relay, and the relay
    // moves between outputs as they come and go.
    let (player, queue) = Player::new();
    let mut output = Output::new(Relay::new(queue), lost_tx);

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

    let moved = output.check();
    let opened = matches!(moved, Moved::To { .. });
    for event in output.events(&moved, true) {
        let _ = events.send(event);
    }
    if !opened {
        let _ = events.send(Event::Devices {
            names: output_devices(),
            active: None,
        });
    }

    loop {
        // Wait for a command or a device error, but wake regularly to report
        // position, notice the track ending, and look at the outputs.
        let wake = runtime.block_on(async {
            tokio::select! {
                command = commands.recv() => Some(command),
                Some(opening) = lost_rx.recv() => {
                    output.complained(opening);
                    None
                }
                _ = tokio::time::sleep(POSITION_INTERVAL) => None,
            }
        });
        let command = match wake {
            Some(command) => Ok(command),
            None => Err(()),
        };

        match command {
            Ok(Some(Command::Shutdown)) | Ok(None) => break,
            Ok(Some(Command::Play { track, url })) => {
                player.clear();
                tap.mark_idle();

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
                buffering = None;
                reported_buffer = f64::NEG_INFINITY;
                let _ = events.send(Event::Paused(true));
            }
            Ok(Some(Command::SetVolume(level))) => {
                player.set_volume(level.clamp(0.0, 2.0));
            }

            Ok(Some(Command::RefreshDevices)) => {
                let _ = events.send(Event::Devices {
                    names: output_devices(),
                    active: output.chosen.clone(),
                });
            }

            Ok(Some(Command::SetDevice(name))) => {
                output.chosen = name;
                let moved = output.check();
                if let Moved::Failed {
                    err,
                    nowhere: false,
                } = &moved
                {
                    let _ = events.send(Event::Failed(format!("cannot use that output: {err}")));
                }
                let idle = !playing || player.is_paused();
                for event in output.events(&moved, idle) {
                    let _ = events.send(event);
                }
                // The picker shows the choice even when nothing had to move.
                if !matches!(moved, Moved::To { .. }) {
                    let _ = events.send(Event::Devices {
                        names: output_devices(),
                        active: output.chosen.clone(),
                    });
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

        if output.due() {
            let moved = output.check();
            let idle = !playing || player.is_paused();
            for event in output.events(&moved, idle) {
                let _ = events.send(event);
            }
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

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// With no choice made, playback goes wherever the system default is:
    /// that is how AirPods and a monitor's speakers get it.
    #[test]
    fn with_no_choice_playback_follows_the_default() {
        let here = ids(&["speakers", "airpods"]);
        assert_eq!(
            pick_output(None, &here, Some("airpods")),
            Some("airpods".into())
        );
        assert_eq!(
            pick_output(None, &here, Some("speakers")),
            Some("speakers".into())
        );
    }

    /// A chosen output wins while it is plugged in, and playback falls back
    /// to the default while it is not, to return when it does.
    #[test]
    fn a_chosen_output_is_used_while_it_is_there() {
        let all = ids(&["speakers", "monitor"]);
        assert_eq!(
            pick_output(Some("monitor"), &all, Some("speakers")),
            Some("monitor".into())
        );
        let unplugged = ids(&["speakers"]);
        assert_eq!(
            pick_output(Some("monitor"), &unplugged, Some("speakers")),
            Some("speakers".into())
        );
        assert_eq!(pick_output(Some("monitor"), &[], None), None);
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
mod output_probe {
    use std::time::Duration;

    use rodio::Source;

    use super::*;

    /// Plays silence through the relay and moves it across every output on
    /// this machine, checking playback carries on at each stop. Silence, so
    /// nothing is heard; and it never changes the system default.
    ///
    /// `cargo test moves_between_outputs -- --ignored --nocapture`
    #[test]
    #[ignore = "depends on the host's audio hardware"]
    fn moves_between_outputs() {
        let all = output_devices();
        let default = wanted_output(None).expect("a default output");
        assert!(
            all.iter().any(|(id, _)| *id == default),
            "the default {default} is not among the outputs {all:?}"
        );

        let (player, queue) = Player::new();
        let (lost, _complaints) = mpsc::unbounded_channel();
        let mut output = Output::new(Relay::new(queue), lost);
        player.append(
            rodio::source::Zero::new(
                std::num::NonZero::new(2).unwrap(),
                std::num::NonZero::new(44_100).unwrap(),
            )
            .take_duration(Duration::from_secs(60)),
        );
        player.pause();

        let mut stops: Vec<(Option<String>, String)> = all
            .iter()
            .map(|(id, label)| (Some(id.clone()), label.clone()))
            .collect();
        stops.push((None, "the system default".to_string()));
        for (chosen, label) in stops {
            output.chosen = chosen;
            let moved = output.check();
            let before = player.get_pos();
            player.play();
            std::thread::sleep(Duration::from_millis(600));
            player.pause();
            let played = player.get_pos() - before;
            println!("OUTPUT {label}: {moved:?}, played {played:?}");
            assert!(
                played >= Duration::from_millis(300),
                "playback stalled on {label}"
            );
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
