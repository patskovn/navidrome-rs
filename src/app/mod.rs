//! Application features.

use std::time::Duration;

pub mod albums;
pub mod artists;
pub mod auth;
pub mod entry;
pub mod player;
pub mod songs;
pub mod ui;

/// Which half of a browse tab has the keyboard.
///
/// The search field starts focused and accepts everything except the three
/// reserved keys: Up and Down move between the field and the list, and Tab
/// switches tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneFocus {
    #[default]
    Search,
    List,
}

/// How long to wait after the last keystroke before searching.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Cuts `text` to `width` columns, marking the cut with an ellipsis.
///
/// Counts characters, not bytes, so a title with an accent in it is not sliced
/// mid-codepoint. Callers pad to width themselves where they need it.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}
