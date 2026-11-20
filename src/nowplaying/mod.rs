//! Publishing what is playing to the desktop's own media controls.
//!
//! On macOS that is Now Playing: the Control Centre tile, the lock screen, the
//! media keys, and the play/pause button on a pair of headphones. Elsewhere
//! this is a no-op — the shape is here so `main` never has to say `cfg`.
//!
//! Two facts from the platform shape this module, both verified rather than
//! assumed (see `AGENTS.md`):
//!
//! * Metadata can be published from any thread, but remote *commands* are
//!   delivered on the main thread's run loop only. Hence [`NowPlaying::pump`],
//!   which the event loop calls, and which is the reason this type is not
//!   `Send`.
//! * The system will not show a client that has not declared itself eligible,
//!   and the call that declares it is private. Without it macOS accepts the
//!   registration and then refuses to resolve it, so nothing appears.
//!
//! Everything above `platform` is plain data, so the interesting decision —
//! *when* is a republish worth it — is testable on any machine.

use std::time::Duration;
use std::time::Instant;

use tokio::sync::mpsc;

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "unsupported.rs")]
mod platform;

/// What the system is showing, and what it would extrapolate from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub elapsed: Duration,
    pub playing: bool,
    /// Which artwork belongs on the tile. The image itself arrives later and
    /// separately — see [`ArtworkSlot`].
    pub cover_art: Option<String>,
}

impl Snapshot {
    /// What the transport would put on the lock screen, or `None` when there is
    /// nothing loaded and the tile should go away.
    pub fn of(player: &crate::app::player::State) -> Option<Self> {
        let track = player.current_track()?;
        Some(Self {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration: track
                .duration
                .and_then(|secs| u64::try_from(secs).ok())
                .map(Duration::from_secs),
            elapsed: player.elapsed,
            playing: player.is_playing,
            cover_art: track.cover_art.clone(),
        })
    }

    /// Everything except the clock. Two snapshots that agree here describe the
    /// same tile, and only the position has moved.
    #[allow(clippy::type_complexity)]
    fn identity(
        &self,
    ) -> (
        &str,
        Option<&String>,
        Option<&String>,
        Option<Duration>,
        Option<&String>,
    ) {
        (
            &self.title,
            self.artist.as_ref(),
            self.album.as_ref(),
            self.duration,
            self.cover_art.as_ref(),
        )
    }
}

/// The tile's picture: which one was asked for, and which one is in hand.
///
/// Artwork is fetched over the network while the tile is already up, so it
/// can land after the track it was for has been skipped. Two rules follow.
/// A picture only goes on the tile whose `cover_art` it was fetched for — a
/// late cover from the previous album would be worse than none. And each id is
/// asked for once: consecutive tracks of one album share their artwork, and a
/// failed fetch is not retried on every redraw.
///
/// Generic over the image so the rules can be tested without a platform.
struct ArtworkSlot<A> {
    requested: Option<String>,
    loaded: Option<(String, A)>,
}

impl<A> Default for ArtworkSlot<A> {
    fn default() -> Self {
        Self {
            requested: None,
            loaded: None,
        }
    }
}

impl<A> ArtworkSlot<A> {
    /// The id to fetch for a tile showing `cover_art`, if one is needed.
    fn want(&mut self, cover_art: Option<&str>) -> Option<String> {
        let id = cover_art?;
        if self.requested.as_deref() == Some(id) {
            return None;
        }
        self.requested = Some(id.to_string());
        Some(id.to_string())
    }

    /// Keeps `art` if the tile is still showing what it was fetched for.
    fn land(&mut self, showing: Option<&str>, id: &str, art: A) -> bool {
        if showing != Some(id) {
            return false;
        }
        self.loaded = Some((id.to_string(), art));
        true
    }

    /// The picture for a tile showing `cover_art`, if it is in hand.
    fn for_tile(&self, cover_art: Option<&str>) -> Option<&A> {
        match &self.loaded {
            Some((id, art)) if Some(id.as_str()) == cover_art => Some(art),
            _ => None,
        }
    }
}

/// A button pressed somewhere the app does not own: Control Centre, a media
/// key, a headset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}

/// How far the published position may drift before it is worth re-sending.
///
/// The system extrapolates from the position and rate it was last given, so a
/// steadily playing track needs no updates at all. Only a jump — a seek, a
/// restart, a track change — has to be corrected.
const DRIFT: Duration = Duration::from_secs(2);

/// The last thing handed to the system, and when.
struct Published {
    snapshot: Snapshot,
    at: Instant,
}

impl Published {
    /// Whether `next` says something the system does not already know.
    fn stale(&self, next: &Snapshot, now: Instant) -> bool {
        if self.snapshot.identity() != next.identity() || self.snapshot.playing != next.playing {
            return true;
        }
        // Where the system believes the playhead is by now.
        let expected = if self.snapshot.playing {
            self.snapshot.elapsed + now.duration_since(self.at)
        } else {
            self.snapshot.elapsed
        };
        expected.abs_diff(next.elapsed) > DRIFT
    }
}

/// Handle onto the desktop's media controls.
///
/// Deliberately not `Send`: [`pump`](Self::pump) is only meaningful on the
/// thread the process started on.
pub struct NowPlaying {
    published: Option<Published>,
    artwork: ArtworkSlot<platform::Artwork>,
    backend: platform::Backend,
}

impl NowPlaying {
    /// Registers with the system and returns the stream of remote commands.
    ///
    /// Must be called on the main thread. Failure is not an error worth
    /// stopping for: the receiver simply stays silent and the app is exactly as
    /// useful as it was before.
    pub fn start() -> (Self, mpsc::UnboundedReceiver<Remote>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        (
            Self {
                published: None,
                artwork: ArtworkSlot::default(),
                backend: platform::Backend::start(commands),
            },
            receiver,
        )
    }

    /// Tells the system what is playing, if that has changed.
    ///
    /// Returns an artwork id when the tile wants a picture it does not have;
    /// fetch it and hand the bytes to [`set_artwork`](Self::set_artwork).
    pub fn update(&mut self, next: Option<Snapshot>) -> Option<String> {
        let now = Instant::now();
        match next {
            Some(snapshot) => {
                let fresh = match &self.published {
                    Some(published) => published.stale(&snapshot, now),
                    None => true,
                };
                if !fresh {
                    return None;
                }
                let cover_art = snapshot.cover_art.as_deref();
                let wanted = self.artwork.want(cover_art);
                // Every publish replaces the whole dictionary, so the picture
                // has to ride along each time or the next pause would drop it.
                self.backend
                    .set(&snapshot, self.artwork.for_tile(cover_art));
                self.published = Some(Published { snapshot, at: now });
                wanted
            }
            None => {
                if self.published.take().is_some() {
                    self.backend.clear();
                }
                None
            }
        }
    }

    /// Puts fetched artwork on the tile, if it is still for what is showing.
    pub fn set_artwork(&mut self, cover_art: &str, bytes: &[u8]) {
        let Some(published) = &self.published else {
            return;
        };
        let showing = published.snapshot.cover_art.as_deref();
        // Checked before decoding: a stale image is not worth the work.
        if showing != Some(cover_art) {
            return;
        }
        let Some(art) = platform::Artwork::decode(bytes) else {
            log::warn!("undecodable artwork {cover_art}; the tile goes without");
            return;
        };
        if self.artwork.land(showing, cover_art, art) {
            self.backend
                .set(&published.snapshot, self.artwork.for_tile(showing));
        }
    }

    /// Whether anything is on the tile, and so whether commands can arrive.
    pub fn is_published(&self) -> bool {
        self.published.is_some()
    }

    /// Delivers any remote commands the system has queued for us.
    ///
    /// Cheap and non-blocking, and does nothing at all when the queue is empty,
    /// so the event loop can call it every time round.
    pub fn pump(&self) {
        self.backend.pump();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::player;
    use crate::subsonic::Track;

    fn snapshot() -> Snapshot {
        Snapshot {
            title: "Blue in Green".into(),
            artist: Some("Miles Davis".into()),
            album: Some("Kind of Blue".into()),
            duration: Some(Duration::from_secs(337)),
            elapsed: Duration::from_secs(10),
            playing: true,
            cover_art: Some("al-1".into()),
        }
    }

    fn published(snapshot: Snapshot) -> Published {
        Published {
            snapshot,
            at: Instant::now(),
        }
    }

    /// The whole point of the drift rule: a track playing through generates a
    /// position event five times a second and none of them are worth an XPC
    /// round trip, because the system is already counting.
    #[test]
    fn a_steadily_advancing_track_is_not_republished() {
        let published = published(snapshot());
        let now = published.at + Duration::from_secs(5);
        let next = Snapshot {
            elapsed: Duration::from_secs(15),
            ..snapshot()
        };
        assert!(!published.stale(&next, now));
    }

    #[test]
    fn a_seek_is_republished() {
        let published = published(snapshot());
        let now = published.at + Duration::from_secs(5);
        let next = Snapshot {
            elapsed: Duration::from_secs(200),
            ..snapshot()
        };
        assert!(published.stale(&next, now));
    }

    /// Backwards is the case a naive "has it moved enough" check gets wrong:
    /// restarting a track lands *behind* where the system thinks we are.
    #[test]
    fn restarting_a_track_is_republished() {
        let published = published(snapshot());
        let now = published.at + Duration::from_secs(5);
        let next = Snapshot {
            elapsed: Duration::ZERO,
            ..snapshot()
        };
        assert!(published.stale(&next, now));
    }

    #[test]
    fn pausing_is_republished() {
        let published = published(snapshot());
        let now = published.at;
        let next = Snapshot {
            playing: false,
            ..snapshot()
        };
        assert!(published.stale(&next, now));
    }

    /// A paused tile does not advance, so the same position stays correct
    /// however long the pause lasts.
    #[test]
    fn a_long_pause_needs_no_correction() {
        let published = published(Snapshot {
            playing: false,
            ..snapshot()
        });
        let now = published.at + Duration::from_secs(600);
        let next = Snapshot {
            playing: false,
            ..snapshot()
        };
        assert!(!published.stale(&next, now));
    }

    #[test]
    fn a_new_track_is_republished_even_at_the_same_position() {
        let published = published(snapshot());
        let now = published.at;
        let next = Snapshot {
            title: "All Blues".into(),
            ..snapshot()
        };
        assert!(published.stale(&next, now));
    }

    /// Another album's artwork is another tile, even if the tags match.
    #[test]
    fn new_artwork_is_republished() {
        let published = published(snapshot());
        let next = Snapshot {
            cover_art: Some("al-2".into()),
            ..snapshot()
        };
        assert!(published.stale(&next, published.at));
    }

    #[test]
    fn artwork_is_asked_for_once_per_id() {
        let mut slot = ArtworkSlot::<u8>::default();
        assert_eq!(slot.want(Some("al-1")), Some("al-1".into()));
        // The next track of the same album, or a pause: nothing new to fetch,
        // including after a failed fetch that never landed.
        assert_eq!(slot.want(Some("al-1")), None);
        assert_eq!(slot.want(Some("al-2")), Some("al-2".into()));
        assert_eq!(slot.want(None), None, "untagged tracks fetch nothing");
    }

    /// The regression this guards: skip a track while its cover downloads, and
    /// the old cover lands on the new album's tile.
    #[test]
    fn late_artwork_for_a_skipped_album_is_dropped() {
        let mut slot = ArtworkSlot::<u8>::default();
        slot.want(Some("al-1"));
        slot.want(Some("al-2"));
        assert!(!slot.land(Some("al-2"), "al-1", 1));
        assert_eq!(slot.for_tile(Some("al-2")), None);

        assert!(slot.land(Some("al-2"), "al-2", 2));
        assert_eq!(slot.for_tile(Some("al-2")), Some(&2));
    }

    /// A picture in hand is only offered to the tile it belongs to — moving to
    /// another album must not keep showing the previous cover meanwhile.
    #[test]
    fn artwork_in_hand_is_not_shown_on_another_album() {
        let mut slot = ArtworkSlot::<u8>::default();
        slot.want(Some("al-1"));
        slot.land(Some("al-1"), "al-1", 1);
        assert_eq!(slot.for_tile(Some("al-1")), Some(&1));
        assert_eq!(slot.for_tile(Some("al-2")), None);
        assert_eq!(slot.for_tile(None), None);
    }

    #[test]
    fn nothing_playing_has_no_snapshot() {
        assert_eq!(Snapshot::of(&player::State::new()), None);
    }

    #[test]
    fn a_snapshot_carries_the_tags_the_tile_shows() {
        let state = player::State {
            queue: vec![Track {
                title: "So What".into(),
                artist: Some("Miles Davis".into()),
                album: Some("Kind of Blue".into()),
                duration: Some(545),
                cover_art: Some("al-1".into()),
                ..Default::default()
            }],
            current: Some(0),
            is_playing: true,
            elapsed: Duration::from_secs(30),
            ..player::State::new()
        };

        let snapshot = Snapshot::of(&state).unwrap();
        assert_eq!(snapshot.title, "So What");
        assert_eq!(snapshot.artist.as_deref(), Some("Miles Davis"));
        assert_eq!(snapshot.album.as_deref(), Some("Kind of Blue"));
        assert_eq!(snapshot.duration, Some(Duration::from_secs(545)));
        assert_eq!(snapshot.elapsed, Duration::from_secs(30));
        assert!(snapshot.playing);
        assert_eq!(snapshot.cover_art.as_deref(), Some("al-1"));
    }

    /// A server that does not report length must not produce a nonsense
    /// duration; the tile just draws no scrubber.
    #[test]
    fn a_track_of_unknown_length_publishes_no_duration() {
        let state = player::State {
            queue: vec![Track {
                title: "Untagged".into(),
                duration: None,
                ..Default::default()
            }],
            current: Some(0),
            ..player::State::new()
        };
        assert_eq!(Snapshot::of(&state).unwrap().duration, None);
    }
}
