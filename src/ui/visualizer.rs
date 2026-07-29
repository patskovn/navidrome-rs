//! The "jumping bars": a spectrum analyser fed by the audio tap.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};

use crate::audio::SampleTap;

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

/// How fast peak-hold caps sink, as a fraction of full height per frame. Slow
/// enough to read as a falling marker, fast enough not to look stuck.
const PEAK_FALL: f32 = 0.012;

/// Retained bar heights, so the visualiser can ease between frames.
///
/// Lives in the widget layer, not reducer state: it is presentation-only and
/// changes at frame rate.
#[derive(Debug, Default, Clone)]
pub struct Spectrum {
    levels: Vec<f32>,
    /// Highest recent level per bar, sinking slowly. With no colour available
    /// to signal intensity, these caps carry most of the visual information.
    peaks: Vec<f32>,
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
            self.peaks.clear();
            return &self.levels;
        }
        if self.levels.len() != bars {
            self.levels = vec![0.0; bars];
            self.peaks = vec![0.0; bars];
        }

        let Some((samples, sample_rate)) = tap.snapshot() else {
            // Nothing playing: ease everything down rather than freezing.
            for level in &mut self.levels {
                *level *= 1.0 - DECAY;
            }
            self.sink_peaks();
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

        self.sink_peaks();
        &self.levels
    }

    /// Lowers each cap towards its bar, snapping straight up when the bar
    /// overtakes it.
    fn sink_peaks(&mut self) {
        for (peak, level) in self.peaks.iter_mut().zip(self.levels.iter()) {
            if *level >= *peak {
                *peak = *level;
            } else {
                *peak = (*peak - PEAK_FALL).max(*level);
            }
        }
    }

    pub fn levels(&self) -> &[f32] {
        &self.levels
    }

    pub fn peaks(&self) -> &[f32] {
        &self.peaks
    }
}

fn log_bucket(bar: usize, bars: usize, max_index: f32) -> f32 {
    let t = bar as f32 / bars as f32;
    // Exponential curve across the buckets, so low frequencies get
    // proportionally more bars than high ones.
    max_index.powf(t) - 1.0
}

/// Cap drawn above a bar at its recent peak.
const CAP: char = '▔';

/// Renders bar heights as a monochrome column chart filling `area`.
///
/// Deliberately sets no foreground colour: the bars inherit the terminal's
/// own, so the visualiser reads correctly on light and dark themes alike.
/// Intensity is carried by weight (dim body, bold tip) and by the peak caps
/// rather than by hue.
pub struct SpectrumWidget<'a> {
    levels: &'a [f32],
    peaks: &'a [f32],
}

impl<'a> SpectrumWidget<'a> {
    pub fn new(levels: &'a [f32], peaks: &'a [f32]) -> Self {
        Self { levels, peaks }
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
            let top_row = eighths.saturating_sub(1) / 8;

            for row in 0..rows {
                // Row 0 is the bottom of the chart.
                let filled = eighths.saturating_sub(row * 8).min(8);
                let ch = BLOCKS[filled];
                if ch == ' ' {
                    continue;
                }
                // Weight stands in for colour: the body recedes, the tip leads.
                let style = if row == top_row {
                    Style::new().bold()
                } else if row * 3 < rows {
                    Style::new().dim()
                } else {
                    Style::new()
                };
                let y = area.y + (rows - 1 - row) as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_style(style);
                }
            }

            // Peak cap, drawn only where it has separated from the bar so it
            // never sits on top of the tip it is tracking.
            let Some(peak) = self.peaks.get(column) else {
                continue;
            };
            let peak_row = ((peak.clamp(0.0, 1.0) * rows as f32).ceil() as usize).saturating_sub(1);
            if peak_row > top_row || eighths == 0 {
                let y = area.y + (rows - 1 - peak_row) as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(CAP);
                    cell.set_style(Style::new().dim());
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

    /// The whole point of the monochrome design: no cell may carry a colour,
    /// so the bars inherit the terminal's foreground on any theme.
    #[test]
    fn nothing_rendered_sets_a_foreground_colour() {
        use ratatui::buffer::Buffer;
        use ratatui::style::Color;

        let levels = [0.9, 0.5, 0.2, 0.05, 0.0];
        let peaks = [0.95, 0.8, 0.7, 0.6, 0.4];
        let area = Rect::new(0, 0, 5, 6);
        let mut buf = Buffer::empty(area);

        SpectrumWidget::new(&levels, &peaks).render(area, &mut buf);

        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let style = buf.cell((x, y)).unwrap().style();
                // `Reset` is ratatui's "inherit the terminal's colour", which
                // is precisely what a colourless visualiser should leave behind.
                let uncoloured = |c: Option<Color>| matches!(c, None | Some(Color::Reset));
                assert!(uncoloured(style.fg), "cell ({x},{y}) set fg {:?}", style.fg);
                assert!(uncoloured(style.bg), "cell ({x},{y}) set bg {:?}", style.bg);
            }
        }
    }

    #[test]
    fn a_bar_renders_dim_body_and_bold_tip() {
        use ratatui::buffer::Buffer;
        use ratatui::style::Modifier;

        let levels = [1.0];
        let peaks = [1.0];
        let area = Rect::new(0, 0, 1, 6);
        let mut buf = Buffer::empty(area);
        SpectrumWidget::new(&levels, &peaks).render(area, &mut buf);

        // Top row of a full-height bar is the tip.
        let tip = buf.cell((0, area.y)).unwrap().style();
        assert!(
            tip.add_modifier.contains(Modifier::BOLD),
            "tip should be bold"
        );

        // Bottom row is body, which recedes.
        let base = buf.cell((0, area.bottom() - 1)).unwrap().style();
        assert!(
            base.add_modifier.contains(Modifier::DIM),
            "base should be dim"
        );
    }

    #[test]
    fn peaks_hold_above_a_falling_bar_then_sink() {
        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new();

        // Establish a loud bar, then let it fall silent.
        let tone: Vec<f32> = (0..WINDOW)
            .map(|i| ((i as f32 / 44_100.0) * 220.0 * std::f32::consts::TAU).sin())
            .collect();
        feed(&tap, tone);
        for _ in 0..10 {
            spectrum.update(&tap, 32);
        }
        let held = spectrum.peaks().iter().cloned().fold(0.0f32, f32::max);
        assert!(held > 0.1, "a peak should have been captured");

        tap.mark_idle();
        for _ in 0..5 {
            spectrum.update(&tap, 32);
        }
        let after = spectrum.peaks().iter().cloned().fold(0.0f32, f32::max);
        let bars = spectrum.levels().iter().cloned().fold(0.0f32, f32::max);
        assert!(after < held, "caps should sink once the bar falls");
        assert!(after > bars, "caps should still float above the bar");

        // And eventually they land.
        for _ in 0..400 {
            spectrum.update(&tap, 32);
        }
        assert!(
            spectrum.peaks().iter().all(|p| *p < 0.01),
            "caps should settle to the floor in silence"
        );
    }

    /// Prints a frame so the monochrome look can be eyeballed:
    /// `cargo test preview_frame -- --nocapture --ignored`
    #[test]
    #[ignore = "visual check only"]
    fn preview_frame() {
        use ratatui::buffer::Buffer;

        let mut spectrum = Spectrum::new();
        let tap = SampleTap::new();
        // A chord plus some noise, so the bars are not all one height.
        let mix: Vec<f32> = (0..WINDOW)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                let tau = std::f32::consts::TAU;
                0.5 * (t * 110.0 * tau).sin()
                    + 0.35 * (t * 440.0 * tau).sin()
                    + 0.2 * (t * 1800.0 * tau).sin()
                    + 0.1 * (t * 5000.0 * tau).sin()
            })
            .collect();
        feed(&tap, mix);
        for _ in 0..12 {
            spectrum.update(&tap, 48);
        }
        // Let the bars drop so the peak caps separate and become visible.
        tap.mark_idle();
        for _ in 0..6 {
            spectrum.update(&tap, 48);
        }

        let area = Rect::new(0, 0, 48, 8);
        let mut buf = Buffer::empty(area);
        SpectrumWidget::new(spectrum.levels(), spectrum.peaks()).render(area, &mut buf);

        println!();
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| {
                    buf.cell((x, y))
                        .unwrap()
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                })
                .collect();
            println!("|{row}|");
        }
        println!();
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
        assert!(
            peak > 0.1,
            "a pure tone should raise a bar, peak was {peak}"
        );
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
