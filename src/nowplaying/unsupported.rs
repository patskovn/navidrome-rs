//! Everywhere that is not macOS.
//!
//! MPRIS on Linux would slot in here — same three methods, a D-Bus object
//! instead of an Objective-C one.

use tokio::sync::mpsc;

use super::Remote;
use super::Snapshot;

pub struct Backend;

/// Nothing to draw it on; kept so the shared logic reads the same everywhere.
pub struct Artwork;

impl Artwork {
    pub fn decode(_bytes: &[u8]) -> Option<Self> {
        Some(Self)
    }
}

impl Backend {
    pub fn start(_commands: mpsc::UnboundedSender<Remote>) -> Self {
        Self
    }

    pub fn set(&self, _snapshot: &Snapshot, _artwork: Option<&Artwork>) {}

    pub fn clear(&self) {}

    pub fn pump(&self) {}
}
