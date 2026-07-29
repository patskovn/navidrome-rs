use ratatui::style::Color;
use ratatui::style::Style;

/// A single place for colours, so the palette can be changed without hunting
/// through widgets.
pub struct Theme;

impl Theme {
    pub const ACCENT: Color = Color::Magenta;
    pub const FOCUSED_BORDER: Color = Color::Green;
    pub const DIM: Color = Color::DarkGray;
    pub const ERROR: Color = Color::Red;
    pub const PLAYING: Color = Color::Cyan;

    /// Border style for a pane, green when it holds focus.
    pub fn border(is_focused: bool) -> Style {
        if is_focused {
            Style::new().fg(Self::FOCUSED_BORDER)
        } else {
            Style::default()
        }
    }

    pub fn selection() -> Style {
        Style::new().fg(Self::ACCENT).bold().reversed()
    }

    pub fn dim() -> Style {
        Style::new().fg(Self::DIM)
    }
}
