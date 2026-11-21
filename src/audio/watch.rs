//! Word from the system when its audio outputs change.
//!
//! On macOS, CoreAudio says when the default output moves or a device comes
//! or goes, so the engine can sit still until something happens. Elsewhere
//! there is no such word here yet, and the engine looks every second instead.

#[cfg(not(target_os = "macos"))]
pub use elsewhere::OutputWatch;
#[cfg(target_os = "macos")]
pub use macos::OutputWatch;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        AudioObjectAddPropertyListener, AudioObjectID, AudioObjectPropertyAddress,
        AudioObjectPropertySelector, AudioObjectRemovePropertyListener,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    };
    use tokio::sync::mpsc::UnboundedSender;

    /// What is worth hearing about: where the default output is (AirPods
    /// in, a monitor unplugged), and which devices exist (a chosen output
    /// being plugged back in).
    const WATCHED: [AudioObjectPropertySelector; 2] = [
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDevices,
    ];

    const SYSTEM: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

    /// Listening for changes to the system's outputs. Dropping it stops.
    pub struct OutputWatch {
        /// Where a change is sent. Boxed, because its address is what
        /// CoreAudio holds, and it must not move while a listener has it.
        changed: Box<UnboundedSender<()>>,
        registered: Vec<AudioObjectPropertyAddress>,
    }

    impl OutputWatch {
        /// Starts listening, or `None` if CoreAudio refuses: the engine then
        /// looks for itself.
        pub fn start(changed: UnboundedSender<()>) -> Option<Self> {
            let mut watch = Self {
                changed: Box::new(changed),
                registered: Vec::new(),
            };
            for selector in WATCHED {
                let address = AudioObjectPropertyAddress {
                    mSelector: selector,
                    mScope: kAudioObjectPropertyScopeGlobal,
                    mElement: kAudioObjectPropertyElementMain,
                };
                // SAFETY: the client data is the boxed sender, which outlives
                // the listener: `drop` removes every listener before the box
                // goes. CoreAudio copies the address.
                let status = unsafe {
                    AudioObjectAddPropertyListener(
                        SYSTEM,
                        NonNull::from(&address),
                        Some(on_change),
                        watch.client_data(),
                    )
                };
                if status != 0 {
                    log::warn!("cannot watch audio outputs (OSStatus {status}); polling");
                    // Dropping `watch` removes whatever did register.
                    return None;
                }
                watch.registered.push(address);
            }
            Some(watch)
        }

        fn client_data(&self) -> *mut c_void {
            std::ptr::from_ref::<UnboundedSender<()>>(&self.changed)
                .cast_mut()
                .cast()
        }
    }

    impl Drop for OutputWatch {
        fn drop(&mut self) {
            for address in &self.registered {
                // SAFETY: the same listener, address and client data that
                // were registered, removed while the sender is still alive.
                unsafe {
                    AudioObjectRemovePropertyListener(
                        SYSTEM,
                        NonNull::from(address),
                        Some(on_change),
                        self.client_data(),
                    );
                }
            }
        }
    }

    /// Called by CoreAudio, on a thread of its own, when a watched property
    /// changes. Says only that something did: the engine works out what.
    unsafe extern "C-unwind" fn on_change(
        _: AudioObjectID,
        _: u32,
        _: NonNull<AudioObjectPropertyAddress>,
        client: *mut c_void,
    ) -> i32 {
        // SAFETY: `client` is the boxed sender from `start`, alive until the
        // listener is removed.
        let changed = unsafe { &*client.cast::<UnboundedSender<()>>() };
        let _ = changed.send(());
        0
    }
}

#[cfg(all(test, target_os = "macos"))]
mod probe {
    use std::ffi::CStr;
    use std::ptr::NonNull;
    use std::time::Duration;

    use objc2_core_audio::{
        AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice,
        kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
        kAudioAggregateDeviceUIDKey,
    };
    use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};

    use super::OutputWatch;

    fn key(name: &CStr) -> objc2_core_foundation::CFRetained<CFString> {
        CFString::from_str(name.to_str().expect("an ASCII key"))
    }

    /// CoreAudio really does call: a device appearing is reported.
    ///
    /// The device is a private aggregate, which only this process can see,
    /// so nothing else on the machine notices and the default never moves.
    /// `cargo test the_system_says_when_devices_change -- --ignored`
    #[test]
    #[ignore = "depends on the host's audio system"]
    fn the_system_says_when_devices_change() {
        let (changed, mut heard) = tokio::sync::mpsc::unbounded_channel();
        let watch = OutputWatch::start(changed).expect("CoreAudio takes the listener");

        let keys = [
            key(kAudioAggregateDeviceNameKey),
            key(kAudioAggregateDeviceUIDKey),
            key(kAudioAggregateDeviceIsPrivateKey),
        ];
        let name = CFString::from_str("navidrome-rs probe");
        let uid = CFString::from_str("rs.navidrome.probe");
        let values: [&CFType; 3] = [&name, &uid, CFBoolean::new(true)];
        let description = CFDictionary::<CFString, CFType>::from_slices(
            &keys.iter().map(|k| &**k).collect::<Vec<_>>(),
            &values,
        );
        let mut device = 0;
        // SAFETY: a CF dictionary of CF strings to CF values, as documented,
        // and a valid place for the new device's id.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(description.as_opaque(), NonNull::from(&mut device))
        };
        assert_eq!(status, 0, "could not create the probe device");

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut said = false;
        while !said && std::time::Instant::now() < deadline {
            said = heard.try_recv().is_ok();
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: the device just created, destroyed once.
        unsafe { AudioHardwareDestroyAggregateDevice(device) };
        drop(watch);
        assert!(said, "a device appeared and nothing was said");
    }
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use tokio::sync::mpsc::UnboundedSender;

    /// No word from the system here: the engine looks for itself.
    pub struct OutputWatch;

    impl OutputWatch {
        pub fn start(_changed: UnboundedSender<()>) -> Option<Self> {
            None
        }
    }
}
