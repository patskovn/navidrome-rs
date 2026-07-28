//! A pass-through [`Source`] that copies samples aside for the visualiser.
//!
//! The buffer deliberately does **not** live in reducer state. It is written
//! from the audio thread thousands of times a second, so putting it in `State`
//! would mean interior mutability there — which breaks `tca::TestStore`'s
//! expected-vs-actual comparison, exactly as `tgpt`'s `SharedFocus` did.
//! Instead the UI reads it directly at draw time.

use std::sync::{Arc, Mutex};

use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Number of samples kept for analysis. A power of two, because the FFT wants
/// one; 2048 at 44.1kHz is ~46ms, short enough to feel reactive.
pub const WINDOW: usize = 2048;

#[derive(Debug)]
struct Ring {
    samples: Box<[f32; WINDOW]>,
    /// Next write position.
    head: usize,
    /// Sample rate of whatever is currently playing, for FFT bucketing.
    sample_rate: u32,
    /// Set while audio is actually flowing, so the UI can fade the bars out
    /// when playback stops rather than freezing on the last frame.
    active: bool,
}

/// Cheap handle to the tapped samples. Clone freely.
#[derive(Debug, Clone)]
pub struct SampleTap {
    ring: Arc<Mutex<Ring>>,
}

impl Default for SampleTap {
    fn default() -> Self {
        Self::new()
    }
}

impl SampleTap {
    pub fn new() -> Self {
        Self {
            ring: Arc::new(Mutex::new(Ring {
                samples: Box::new([0.0; WINDOW]),
                head: 0,
                sample_rate: 44_100,
                active: false,
            })),
        }
    }

    /// Crate-visible so tests can drive the ring directly; production
    /// writes go through [`Tapped`].
    pub(crate) fn push(&self, sample: f32) {
        if let Ok(mut ring) = self.ring.lock() {
            let head = ring.head;
            ring.samples[head] = sample;
            ring.head = (head + 1) % WINDOW;
            ring.active = true;
        }
    }

    fn set_sample_rate(&self, rate: u32) {
        if let Ok(mut ring) = self.ring.lock() {
            ring.sample_rate = rate;
        }
    }

    /// Marks the tap idle. The visualiser decays to silence rather than
    /// holding the last window on screen.
    pub fn mark_idle(&self) {
        if let Ok(mut ring) = self.ring.lock() {
            ring.active = false;
            ring.samples.fill(0.0);
            ring.head = 0;
        }
    }

    /// Copies the current window out in chronological order, oldest first.
    ///
    /// Returns `None` when nothing is playing, which the visualiser renders as
    /// a flat line.
    pub fn snapshot(&self) -> Option<(Vec<f32>, u32)> {
        let ring = self.ring.lock().ok()?;
        if !ring.active {
            return None;
        }
        let mut out = Vec::with_capacity(WINDOW);
        out.extend_from_slice(&ring.samples[ring.head..]);
        out.extend_from_slice(&ring.samples[..ring.head]);
        Some((out, ring.sample_rate))
    }
}

/// Wraps a [`Source`], forwarding every sample untouched while copying the
/// first channel of each frame into a [`SampleTap`].
pub struct Tapped<S> {
    inner: S,
    tap: SampleTap,
    channels: u16,
    /// Position within the current frame, so we only sample one channel.
    channel_cursor: u16,
}

impl<S: Source> Tapped<S> {
    pub fn new(inner: S, tap: SampleTap) -> Self {
        let channels = inner.channels().get();
        tap.set_sample_rate(inner.sample_rate().get());
        Self {
            inner,
            tap,
            channels,
            channel_cursor: 0,
        }
    }
}

impl<S: Source> Iterator for Tapped<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        // Take one channel per frame: a stereo mix would need both, but for a
        // bar visualiser the left channel is indistinguishable and half the work.
        if self.channel_cursor == 0 {
            self.tap.push(sample);
        }
        self.channel_cursor = (self.channel_cursor + 1) % self.channels.max(1);
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for Tapped<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn idle_tap_yields_no_snapshot() {
        let tap = SampleTap::new();
        assert!(tap.snapshot().is_none());
    }

    #[test]
    fn snapshot_returns_samples_oldest_first() {
        let tap = SampleTap::new();
        for i in 0..WINDOW {
            tap.push(i as f32);
        }
        let (samples, _) = tap.snapshot().expect("tap should be active");

        assert_eq!(samples.len(), WINDOW);
        assert_eq!(samples[0], 0.0, "oldest sample should come first");
        assert_eq!(samples[WINDOW - 1], (WINDOW - 1) as f32);
    }

    #[test]
    fn ring_wraps_and_keeps_only_the_newest_window() {
        let tap = SampleTap::new();
        // Write one and a half windows; the first half should be overwritten.
        for i in 0..(WINDOW + WINDOW / 2) {
            tap.push(i as f32);
        }
        let (samples, _) = tap.snapshot().unwrap();

        assert_eq!(samples.len(), WINDOW);
        assert_eq!(
            samples[WINDOW - 1],
            (WINDOW + WINDOW / 2 - 1) as f32,
            "newest sample should be last"
        );
        assert!(
            samples[0] > 0.0,
            "the oldest samples should have been overwritten"
        );
    }

    #[test]
    fn going_idle_clears_the_window() {
        let tap = SampleTap::new();
        tap.push(1.0);
        assert!(tap.snapshot().is_some());

        tap.mark_idle();
        assert!(tap.snapshot().is_none(), "idle tap should report nothing");
    }
}
