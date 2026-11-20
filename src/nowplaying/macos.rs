//! Now Playing, via `MPNowPlayingInfoCenter` and `MPRemoteCommandCenter`.
//!
//! What is not obvious here, and cost an afternoon of `log stream` to find:
//!
//! * A terminal binary is perfectly welcome to publish. No `.app` bundle, no
//!   `NSApplication`, no window. `mediaremoted` accepts the client and gives it
//!   a bundle identifier synthesised from the executable name.
//! * It will not, however, be *shown*. Control Centre asks for the current
//!   now-playing client and is told "could not find the specified now playing
//!   client" — with or without a signed bundle. The missing step is
//!   [`declare_eligible`], which is private API.
//! * Command handlers are invoked on the main queue. A run loop on a worker
//!   thread receives nothing at all; the main thread must drain the queue,
//!   which is what [`Backend::pump`] does.

use std::ffi::c_void;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSImage;
use objc2_core_foundation::CFRunLoop;
use objc2_core_foundation::CFString;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSData;
use objc2_foundation::NSMutableDictionary;
use objc2_foundation::NSNumber;
use objc2_foundation::NSString;
use objc2_media_player::MPMediaItemArtwork;
use objc2_media_player::MPMediaItemPropertyAlbumTitle;
use objc2_media_player::MPMediaItemPropertyArtist;
use objc2_media_player::MPMediaItemPropertyArtwork;
use objc2_media_player::MPMediaItemPropertyPlaybackDuration;
use objc2_media_player::MPMediaItemPropertyTitle;
use objc2_media_player::MPNowPlayingInfoCenter;
use objc2_media_player::MPNowPlayingInfoPropertyElapsedPlaybackTime;
use objc2_media_player::MPNowPlayingInfoPropertyPlaybackRate;
use objc2_media_player::MPNowPlayingPlaybackState;
use objc2_media_player::MPRemoteCommand;
use objc2_media_player::MPRemoteCommandCenter;
use objc2_media_player::MPRemoteCommandEvent;
use objc2_media_player::MPRemoteCommandHandlerStatus;
use tokio::sync::mpsc;

use super::Remote;
use super::Snapshot;

/// Declares this process eligible to be *the* now-playing application.
///
/// `MRMediaRemoteSetCanBeNowPlayingApplication` is private, so this is done by
/// name at run time and skipped if it ever disappears: the app then plays
/// exactly as it does today, minus the tile. Nothing else in here depends on
/// it, so a future macOS that drops the symbol costs a feature, not a crash.
///
/// Without this call the registration still succeeds and `mediaremoted` still
/// tracks the state — Control Centre simply never resolves the client, which
/// looks from the outside like the API silently doing nothing.
fn declare_eligible() -> bool {
    const FRAMEWORK: &std::ffi::CStr =
        c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote";
    const SYMBOL: &std::ffi::CStr = c"MRMediaRemoteSetCanBeNowPlayingApplication";

    unsafe {
        let library = libc::dlopen(FRAMEWORK.as_ptr(), libc::RTLD_NOW);
        if library.is_null() {
            log::warn!("MediaRemote is not loadable; Now Playing will not appear");
            return false;
        }
        let symbol = libc::dlsym(library, SYMBOL.as_ptr());
        if symbol.is_null() {
            log::warn!("{SYMBOL:?} is gone; Now Playing will not appear");
            return false;
        }
        let declare: extern "C" fn(bool) = std::mem::transmute::<*mut c_void, _>(symbol);
        declare(true);
        true
    }
}

/// A decoded cover, ready to hang on the tile.
pub struct Artwork(Retained<MPMediaItemArtwork>);

impl Artwork {
    /// Decodes JPEG/PNG bytes as the server sent them. `None` for anything
    /// AppKit cannot read, which costs the tile its picture and nothing else.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
        let mut size = image.size();
        // Some encodings report no intrinsic size; the system needs a nonzero
        // bound to lay the image out, and scales it either way.
        if size.width <= 0.0 || size.height <= 0.0 {
            size = CGSize::new(256.0, 256.0);
        }

        // The system asks for the image lazily, possibly at a size of its own
        // choosing, and may do so more than once. One image serves every
        // request; the block owns it, and the artwork owns the block, so the
        // pointer handed back stays valid as long as anyone can ask.
        let handler =
            RcBlock::new(move |_requested: CGSize| -> NonNull<NSImage> { NonNull::from(&*image) });
        let artwork = unsafe {
            MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                MPMediaItemArtwork::alloc(),
                size,
                &handler,
            )
        };
        Some(Self(artwork))
    }
}

pub struct Backend {
    /// The registered handlers. Dropping these unregisters them, so they are
    /// held for the life of the process rather than being leaked.
    _handlers: Vec<
        RcBlock<dyn Fn(core::ptr::NonNull<MPRemoteCommandEvent>) -> MPRemoteCommandHandlerStatus>,
    >,
}

impl Backend {
    pub fn start(commands: mpsc::UnboundedSender<Remote>) -> Self {
        declare_eligible();

        let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
        let supported: [(Remote, Retained<MPRemoteCommand>); 5] = unsafe {
            [
                (Remote::Play, center.playCommand()),
                (Remote::Pause, center.pauseCommand()),
                (Remote::Toggle, center.togglePlayPauseCommand()),
                (Remote::Next, center.nextTrackCommand()),
                (Remote::Previous, center.previousTrackCommand()),
            ]
        };

        let mut handlers = Vec::with_capacity(supported.len());
        for (remote, command) in supported {
            let commands = commands.clone();
            let handler = RcBlock::new(move |_event: core::ptr::NonNull<MPRemoteCommandEvent>| {
                // A dead receiver means the app is shutting down; the
                // system is told the command succeeded either way, because
                // reporting failure makes Control Centre grey the button.
                let _ = commands.send(remote);
                MPRemoteCommandHandlerStatus::Success
            });
            unsafe {
                command.setEnabled(true);
                command.addTargetWithHandler(&handler);
            }
            handlers.push(handler);
        }

        // Anything left enabled but unhandled draws a control that does
        // nothing. Scrubbing is the one users reach for, so say plainly that
        // it is not offered — seeking exists in the engine but is not bound.
        unsafe {
            center.changePlaybackPositionCommand().setEnabled(false);
            center.seekForwardCommand().setEnabled(false);
            center.seekBackwardCommand().setEnabled(false);
            center.skipForwardCommand().setEnabled(false);
            center.skipBackwardCommand().setEnabled(false);
        }

        Self {
            _handlers: handlers,
        }
    }

    pub fn set(&self, snapshot: &Snapshot, artwork: Option<&Artwork>) {
        unsafe {
            let info = NSMutableDictionary::<NSString, AnyObject>::new();
            let put = |key: &NSString, value: Retained<AnyObject>| {
                info.setObject_forKey(&value, ProtocolObject::from_ref(key));
            };

            let text = |value: &str| Retained::cast_unchecked(NSString::from_str(value));
            let number = |value: f64| Retained::cast_unchecked(NSNumber::new_f64(value));

            put(MPMediaItemPropertyTitle, text(&snapshot.title));
            if let Some(artist) = &snapshot.artist {
                put(MPMediaItemPropertyArtist, text(artist));
            }
            if let Some(album) = &snapshot.album {
                put(MPMediaItemPropertyAlbumTitle, text(album));
            }
            if let Some(artwork) = artwork {
                put(
                    MPMediaItemPropertyArtwork,
                    Retained::cast_unchecked(artwork.0.clone()),
                );
            }
            if let Some(duration) = snapshot.duration {
                put(
                    MPMediaItemPropertyPlaybackDuration,
                    number(duration.as_secs_f64()),
                );
            }
            put(
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                number(snapshot.elapsed.as_secs_f64()),
            );
            // The rate is how the system counts the clock on from the position
            // above; zero freezes it where it is.
            put(
                MPNowPlayingInfoPropertyPlaybackRate,
                number(if snapshot.playing { 1.0 } else { 0.0 }),
            );

            let center = MPNowPlayingInfoCenter::defaultCenter();
            center.setNowPlayingInfo(Some(&Retained::cast_unchecked(info)));
            // macOS does not infer this from the audio session the way iOS
            // does; without it the tile shows nothing regardless of the rate.
            center.setPlaybackState(if snapshot.playing {
                MPNowPlayingPlaybackState::Playing
            } else {
                MPNowPlayingPlaybackState::Paused
            });
        }
    }

    pub fn clear(&self) {
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
    }

    pub fn pump(&self) {
        // Zero seconds, returning after one source: drain what is waiting and
        // hand the thread straight back to the event loop.
        let mode = CFString::from_static_str("kCFRunLoopDefaultMode");
        CFRunLoop::run_in_mode(Some(&mode), 0.0, true);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// The handler must hand back a live image, at the bounds the cover
    /// reported — this is the one `unsafe` path the shared tests cannot reach.
    #[test]
    fn a_cover_becomes_artwork_the_system_can_draw() {
        let artwork = Artwork::decode(&png(300, 200)).expect("a PNG should decode");
        let bounds = unsafe { artwork.0.bounds() };
        assert_eq!((bounds.size.width, bounds.size.height), (300.0, 200.0));

        // Ask twice, as the system may: the image must survive the first.
        for _ in 0..2 {
            let image = unsafe { artwork.0.imageWithSize(CGSize::new(64.0, 64.0)) };
            assert!(image.is_some(), "the request handler returned nothing");
        }
    }

    #[test]
    fn garbage_is_no_artwork_rather_than_a_crash() {
        assert!(Artwork::decode(b"not an image").is_none());
        assert!(Artwork::decode(&[]).is_none());
    }
}
