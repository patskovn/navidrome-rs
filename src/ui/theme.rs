use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use ratatui::style::Color;
use ratatui::style::Style;

/// Whether the terminal window itself holds focus.
///
/// Deliberately not a reducer `State` field read by every widget: the accent is
/// used from a dozen draw sites and threading a flag through all of them would
/// buy nothing. `entry::State::focused` stays the source of truth and `ui::draw`
/// copies it here once per frame — the same shape as the sample tap, which the
/// visualiser reads at draw time rather than storing.
///
/// True by default, so a terminal that never reports focus (Terminal.app, or
/// tmux without `focus-events on`) simply stays bright.
static FOCUSED: AtomicBool = AtomicBool::new(true);

/// A single place for colours, so the palette can be changed without hunting
/// through widgets.
pub struct Theme;

impl Theme {
    pub const ACCENT: Color = Color::Magenta;
    pub const FOCUSED_BORDER: Color = Color::Green;
    pub const DIM: Color = Color::DarkGray;
    pub const ERROR: Color = Color::Red;
    pub const PLAYING: Color = Color::Cyan;

    /// Called once per frame from `ui::draw`.
    pub fn set_focused(focused: bool) {
        FOCUSED.store(focused, Ordering::Relaxed);
    }

    pub fn is_focused() -> bool {
        FOCUSED.load(Ordering::Relaxed)
    }

    /// The accent, faint while another window has the terminal.
    ///
    /// Faintness is the SGR attribute rather than a second, darker colour: the
    /// 16-colour palette holds no shade below `Magenta`, and hard-coding an RGB
    /// value would override whatever palette the user actually chose.
    pub fn accent() -> Style {
        let style = Style::new().fg(Self::ACCENT);
        if Self::is_focused() {
            style
        } else {
            style.dim()
        }
    }

    /// Border style for a pane, green when it holds focus.
    pub fn border(is_focused: bool) -> Style {
        if is_focused {
            Style::new().fg(Self::FOCUSED_BORDER)
        } else {
            Style::default()
        }
    }

    pub fn selection() -> Style {
        Self::accent().bold().reversed()
    }

    pub fn dim() -> Style {
        Style::new().fg(Self::DIM)
    }
}
