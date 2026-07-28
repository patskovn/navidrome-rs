//! Application features.

use std::time::Duration;

pub mod albums;
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
