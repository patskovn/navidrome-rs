//! The player's output, movable between devices.
//!
//! A rodio `Player` plays into a queue that a device drains. Tie the two
//! together and the player lives only as long as the device: unplug a monitor
//! or put AirPods in, and keeping the music going means a new player,
//! re-opening the track (re-tuning a station, re-downloading a song), seeking
//! back to where it was, and forgetting anything that was paused.
//!
//! So the queue sits behind a [`Relay`] instead, and each device gets an
//! [`Outlet`] onto it. Moving to another output opens the new device, hands it
//! an outlet, and lets the old one go. The track, its position, pause, volume
//! and any download in flight never notice.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rodio::queue::SourcesQueueOutput;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// One source, shared between the devices it may play on, one at a time.
pub struct Relay<S = SourcesQueueOutput>(Arc<Mutex<Shared<S>>>);

struct Shared<S> {
    source: S,
    /// Which outlet may play. Each new outlet takes the turn and every older
    /// one ends: two devices taking turns at the samples would each play
    /// every other one.
    turn: u64,
}

impl<S: Source> Relay<S> {
    pub fn new(source: S) -> Self {
        Self(Arc::new(Mutex::new(Shared { source, turn: 0 })))
    }

    /// An outlet for a newly opened device. From now on it plays, and every
    /// earlier outlet finishes the frame it is in and ends.
    pub fn outlet(&self) -> Outlet<S> {
        let mut shared = lock(&self.0);
        shared.turn += 1;
        Outlet {
            shared: self.0.clone(),
            turn: shared.turn,
            frame: Vec::with_capacity(8),
            next: 0,
        }
    }
}

fn lock<S>(shared: &Mutex<Shared<S>>) -> MutexGuard<'_, Shared<S>> {
    // A panic mid-sample leaves nothing half-written worth refusing over.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One device's view of a [`Relay`].
///
/// It takes whole frames, a sample for every channel, under one lock, so a
/// move between devices can only fall between frames. The new device starts
/// on a left sample, never a right one, and the channels stay where they
/// belong.
pub struct Outlet<S = SourcesQueueOutput> {
    shared: Arc<Mutex<Shared<S>>>,
    turn: u64,
    /// The frame being played, and how much of it has been.
    frame: Vec<Sample>,
    next: usize,
}

impl<S: Source> Iterator for Outlet<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if let Some(&sample) = self.frame.get(self.next) {
            self.next += 1;
            return Some(sample);
        }
        let mut shared = lock(&self.shared);
        if shared.turn != self.turn {
            return None;
        }
        // Read before the frame: at the end of a track the queue already
        // answers for the next one, which is whose frame this will be.
        let channels = shared.source.channels().get();
        self.frame.clear();
        let first = shared.source.next()?;
        self.frame.push(first);
        for _ in 1..channels {
            // Something running dry mid-frame is padded, keeping frames whole.
            self.frame.push(shared.source.next().unwrap_or(0.0));
        }
        self.next = 1;
        Some(first)
    }
}

impl<S: Source> Source for Outlet<S> {
    fn current_span_len(&self) -> Option<usize> {
        // What is left of the frame in hand belongs to the span too.
        let held = self.frame.len() - self.next;
        lock(&self.shared)
            .source
            .current_span_len()
            .map(|len| len + held)
    }

    fn channels(&self) -> ChannelCount {
        lock(&self.shared).source.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        lock(&self.shared).source.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod test {
    use std::num::NonZero;

    use rodio::buffer::SamplesBuffer;

    use super::*;

    /// Stereo frames whose left sample is `n` and right sample is `-n`, so a
    /// sample shows both where it came from and which channel it is on.
    fn numbered(frames: u16) -> SamplesBuffer {
        let samples: Vec<f32> = (1..=frames)
            .flat_map(|n| [f32::from(n), -f32::from(n)])
            .collect();
        SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(44_100).unwrap(),
            samples,
        )
    }

    #[test]
    fn an_outlet_plays_the_source_through() {
        let relay = Relay::new(numbered(3));
        let played: Vec<f32> = relay.outlet().collect();
        assert_eq!(played, [1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
    }

    /// A move mid-frame: the old device finishes its frame and stops, the new
    /// one picks up at the next frame. Nothing lost, nothing played twice,
    /// and the new device starts on a left sample.
    #[test]
    fn a_new_outlet_takes_over_between_frames() {
        let relay = Relay::new(numbered(4));
        let mut old = relay.outlet();
        let before: Vec<f32> = old.by_ref().take(3).collect();
        assert_eq!(before, [1.0, -1.0, 2.0], "partway into frame 2");

        let new = relay.outlet();
        let finished: Vec<f32> = old.collect();
        assert_eq!(finished, [-2.0], "the old outlet ends with its frame");

        let after: Vec<f32> = new.collect();
        assert_eq!(after, [3.0, -3.0, 4.0, -4.0]);
    }

    /// A stereo source that reports, as the player's queue does, how many
    /// samples its span has left.
    struct Remaining(Vec<f32>, usize);

    impl Iterator for Remaining {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            let sample = self.0.get(self.1).copied();
            self.1 += 1;
            sample
        }
    }

    impl Source for Remaining {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.0.len().saturating_sub(self.1))
        }
        fn channels(&self) -> ChannelCount {
            NonZero::new(2).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(44_100).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    /// The span counts down one per sample played, though the outlet takes
    /// two at a time: what it holds still belongs to the span. Otherwise the
    /// device's converter would re-read the format a sample early, mid-frame.
    #[test]
    fn the_span_counts_the_frame_in_hand() {
        let relay = Relay::new(Remaining(vec![0.0; 8], 0));
        let mut outlet = relay.outlet();
        for left in (0..8).rev() {
            outlet.next();
            assert_eq!(outlet.current_span_len(), Some(left));
        }
    }
}
