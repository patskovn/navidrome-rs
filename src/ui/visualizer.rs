//! The "jumping bars": a spectrum analyser fed by the audio tap.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};

use crate::audio::SampleTap;
use crate::ui::theme::Theme;

/// Vertical resolution per cell, using Unicode eighth-blocks so a bar can be
/// eight times more precise than the character grid.
const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Frequencies above this carry little visible energy in most music and would
/// leave the right-hand bars permanently flat.
const MAX_HZ: f32 = 12_000.0;

/// Smoothing applied frame to frame. Without it the bars strobe; too much and
/// they turn to sludge.
const ATTACK: f32 = 0.6;
const DECAY: f32 = 0.25;

/// Retained bar heights, so the visualiser can ease between frames.
///
/// Lives in the widget layer, not reducer state: it is presentation-only and
/// changes at frame rate.
#[derive(Debug, Default, Clone)]
pub struct Spectrum {
    levels: Vec<f32>,
}

impl Spectrum {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recomputes bar heights from the newest audio window.
    ///
    /// `bars` is how many columns are available. Returns normalised heights in
    /// `0.0..=1.0`.
    pub fn update(&mut self, tap: &SampleTap, bars: usize) -> &[f32] {
        if bars == 0 {
            self.levels.clear();
            return &self.levels;
        }
        if self.levels.len() != bars {
            self.levels = vec![0.0; bars];
        }

        let Some((samples, sample_rate)) = tap.snapshot() else {
            // Nothing playing: ease everything down rather than freezing.
            for level in &mut self.levels {
                *level *= 1.0 - DECAY;
            }
            return &self.levels;
        };

        let windowed = hann_window(&samples);
        let spectrum = samples_fft_to_spectrum(
            &windowed,
            sample_rate,
            FrequencyLimit::Max(MAX_HZ.min(sample_rate as f32 / 2.0)),
            Some(&divide_by_N_sqrt),
        );

        let Ok(spectrum) = spectrum else {
            return &self.levels;
        };

        let data = spectrum.data();
        if data.is_empty() {
            return &self.levels;
        }

        // Buckets are laid out logarithmically: musical content bunches into the
        // low end, so linear buckets would leave most bars idle.
        let max_index = data.len() as f32;
        for bar in 0..bars {
            let lo = log_bucket(bar, bars, max_index);
            let hi = log_bucket(bar + 1, bars, max_index).max(lo + 1.0);

            let start = lo as usize;
            let end = (hi as usize).min(data.len());
            if start >= end {
                continue;
            }

            let peak = data[start..end]
                .iter()
                .map(|(_, value)| value.val())
                .fold(0.0f32, f32::max);

            // Log scaling: raw magnitudes span orders of magnitude, so a linear
            // bar would sit at zero except on transients.
            let target = ((1.0 + peak * 400.0).ln() / 6.0).clamp(0.0, 1.0);

            // Rise fast, fall slow — that is what reads as "reacting to music".
            let level = &mut self.levels[bar];
            *level = if target > *level {
                *level + (target - *level) * ATTACK
            } else {
                *level + (target - *level) * DECAY
            };
        }

        &self.levels
    }

    pub fn levels(&self) -> &[f32] {
        &self.levels
    }
}

fn log_bucket(bar: usize, bars: usize, max_index: f32) -> f32 {
    let t = bar as f32 / bars as f32;
    // Exponential curve across the buckets, so low frequencies get
    // proportionally more bars than high ones.
    max_index.powf(t) - 1.0
}

/// Renders bar heights as a column chart filling `area`.
pub struct SpectrumWidget<'a> {
    levels: &'a [f32],
}

impl<'a> SpectrumWidget<'a> {
    pub fn new(levels: &'a [f32]) -> Self {
        Self { levels }
    }
}

impl Widget for SpectrumWidget<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let rows = area.height as usize;

        for (column, level) in self.levels.iter().enumerate().take(area.width as usize) {
            let x = area.x + column as u16;
            // Height in eighths, so bars grow smoothly rather than jumping a
            // whole cell at a time.
            let eighths = (level.clamp(0.0, 1.0) * (rows * 8) as f32).round() as usize;
            let style = Style::new().fg(Theme::spectrum_bar(*level));

            for row in 0..rows {
                // Row 0 is the bottom of the chart.
                let filled = eighths.saturating_sub(row * 8).min(8);
                let ch = BLOCKS[filled];
                if ch == ' ' {
                    continue;
                }
                let y = area.y + (rows - 1 - row) as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_style(style);
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::audio::tap::WINDOW;

    fn feed(tap: &SampleTap, samples: Vec<f32>) {
        for sample in samples {
            tap.push(sample);
        }
    }

    #[test]
    fn silence_decays_towards_zero() {
        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new(); // idle: no snapshot

        spectrum.levels = vec![1.0; 8];
        for _ in 0..60 {
            spectrum.update(&tap, 8);
        }

        assert!(
            spectrum.levels().iter().all(|l| *l < 0.01),
            "bars should fall to silence, got {:?}",
            spectrum.levels()
        );
    }

    #[test]
    fn resizing_reallocates_the_bars() {
        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new();

        assert_eq!(spectrum.update(&tap, 4).len(), 4);
        assert_eq!(spectrum.update(&tap, 16).len(), 16);
        assert_eq!(spectrum.update(&tap, 0).len(), 0);
    }

    #[test]
    fn log_buckets_are_monotonic_and_favour_low_frequencies() {
        let bars = 16;
        let max = 1024.0;
        let buckets: Vec<f32> = (0..=bars).map(|b| log_bucket(b, bars, max)).collect();

        for pair in buckets.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "buckets must not go backwards: {buckets:?}"
            );
        }
        assert!(
            buckets[bars / 2] < max / 4.0,
            "low frequencies should get proportionally more bars"
        );
    }

    #[test]
    fn a_pure_tone_raises_a_bar() {
        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new();

        let tone: Vec<f32> = (0..WINDOW)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                (t * 440.0 * std::f32::consts::TAU).sin()
            })
            .collect();
        feed(&tap, tone);

        for _ in 0..10 {
            spectrum.update(&tap, 32);
        }

        let peak = spectrum.levels().iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.1, "a pure tone should raise a bar, peak was {peak}");
    }

    #[test]
    fn a_tone_lights_low_bars_not_high_ones() {
        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new();

        let tone: Vec<f32> = (0..WINDOW)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                (t * 220.0 * std::f32::consts::TAU).sin()
            })
            .collect();
        feed(&tap, tone);
        for _ in 0..10 {
            spectrum.update(&tap, 32);
        }

        let levels = spectrum.levels();
        let low = levels[..8].iter().cloned().fold(0.0f32, f32::max);
        let high = levels[24..].iter().cloned().fold(0.0f32, f32::max);
        assert!(
            low > high,
            "a 220Hz tone should favour low bars: low={low} high={high}"
        );
    }
}
