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

/// Tests that draw the whole app set the focus flag (through `ui::draw`),
/// and one test checks what the flag does: they all hold this while they
/// touch it, or the parallel runner lets one flip it under another.
#[cfg(test)]
pub fn focus_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A single place for colours, so the palette can be changed without hunting
/// through widgets.
pub struct Theme;

impl Theme {
    pub const ACCENT: Color = Color::Magenta;
    /// The accent with the colour taken out of it, for an unfocused window.
    ///
    /// `Gray`, not the darker `DIM`: the accent also backs the selected row via
    /// `selection()`, which reverses it, and a dark grey background would leave
    /// the row's text nearly unreadable on a dark terminal.
    pub const ACCENT_UNFOCUSED: Color = Color::Gray;
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

    /// The accent, grey while another window has the terminal.
    pub fn accent() -> Style {
        Style::new().fg(if Self::is_focused() {
            Self::ACCENT
        } else {
            Self::ACCENT_UNFOCUSED
        })
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
