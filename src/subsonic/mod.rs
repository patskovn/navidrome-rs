//! The library-server boundary.
//!
//! Reducers talk to the [`Library`] trait, never to `opensubsonic` directly.
//! That keeps the network out of reducer code, so features can be driven by
//! `tca::TestStore` against a fake — the lesson from `tgpt`, where the OpenAI
//! client was constructed inside the reducer and nothing could be tested.

use std::sync::Arc;

use async_trait::async_trait;

pub mod client;

pub use client::OpenSubsonicLibrary;

/// An album as the UI needs it. Deliberately not `opensubsonic`'s type: the app
/// should not have to change shape when the API crate does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub song_count: Option<i64>,
    /// Total running time in seconds.
    pub duration: Option<i64>,
    pub genre: Option<String>,
    pub play_count: Option<i64>,
    /// Id to fetch artwork with; not the image itself.
    pub cover_art: Option<String>,
}

/// A single track.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<i32>,
    /// Length in seconds, when the server knows it.
    pub duration: Option<i64>,
    /// File extension of the original, used to name the cache entry.
    pub suffix: Option<String>,
}

impl Track {
    /// "Artist — Title", falling back gracefully when tags are missing.
    pub fn display(&self) -> String {
        match &self.artist {
            Some(artist) => format!("{artist} — {}", self.title),
            None => self.title.clone(),
        }
    }

    pub fn duration_display(&self) -> String {
        match self.duration {
            Some(secs) if secs >= 0 => format_duration(secs as u64),
            _ => "--:--".to_string(),
        }
    }
}

pub fn format_duration(total_secs: u64) -> String {
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}

/// Results of a `search3` call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchResults {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

/// How albums should be ordered when listing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlbumSort {
    #[default]
    Alphabetical,
    Recent,
    Random,
    Frequent,
}

/// Everything the app needs from a music server.
///
/// Object-safe and `Send + Sync` so it can live in a reducer as
/// `Arc<dyn Library>`.
#[async_trait]
pub trait Library: Send + Sync + 'static {
    /// Verifies credentials. Used by the sign-in screen.
    async fn ping(&self) -> anyhow::Result<()>;

    async fn albums(&self, sort: AlbumSort, offset: u32, limit: u32) -> anyhow::Result<Vec<Album>>;

    /// Tracks of one album, in disc/track order.
    async fn album_tracks(&self, album_id: &str) -> anyhow::Result<Vec<Track>>;

    /// A page of "all music". Subsonic has no true "all songs" endpoint, so
    /// this is served by walking albums — see the implementation.
    async fn all_tracks(&self, offset: u32, limit: u32) -> anyhow::Result<Vec<Track>>;

    async fn search(&self, query: &str) -> anyhow::Result<SearchResults>;

    /// A fully authenticated URL the audio layer can stream from directly.
    fn stream_url(&self, track_id: &str) -> anyhow::Result<String>;
}

/// Shared handle used by reducers.
pub type SharedLibrary = Arc<dyn Library>;

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// In-memory [`Library`] for reducer tests.
    #[derive(Default)]
    pub struct FakeLibrary {
        pub albums: Vec<Album>,
        pub tracks: Vec<Track>,
        pub fail_ping: bool,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeLibrary {
        #[allow(dead_code)]
        pub fn with_albums(albums: Vec<Album>) -> Self {
            Self {
                albums,
                ..Default::default()
            }
        }

        fn record(&self, what: &str) {
            self.calls.lock().expect("poisoned").push(what.to_string());
        }
    }

    #[async_trait]
    impl Library for FakeLibrary {
        async fn ping(&self) -> anyhow::Result<()> {
            self.record("ping");
            if self.fail_ping {
                anyhow::bail!("wrong username or password");
            }
            Ok(())
        }

        async fn albums(
            &self,
            _sort: AlbumSort,
            _offset: u32,
            _limit: u32,
        ) -> anyhow::Result<Vec<Album>> {
            self.record("albums");
            Ok(self.albums.clone())
        }

        async fn album_tracks(&self, _album_id: &str) -> anyhow::Result<Vec<Track>> {
            self.record("album_tracks");
            Ok(self.tracks.clone())
        }

        async fn all_tracks(&self, _offset: u32, _limit: u32) -> anyhow::Result<Vec<Track>> {
            self.record("all_tracks");
            Ok(self.tracks.clone())
        }

        async fn search(&self, _query: &str) -> anyhow::Result<SearchResults> {
            self.record("search");
            Ok(SearchResults {
                albums: self.albums.clone(),
                tracks: self.tracks.clone(),
            })
        }

        fn stream_url(&self, track_id: &str) -> anyhow::Result<String> {
            Ok(format!("https://fake.invalid/stream/{track_id}"))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn track_display_falls_back_without_an_artist() {
        let track = Track {
            title: "Untitled".into(),
            ..Default::default()
        };
        assert_eq!(track.display(), "Untitled");
    }

    #[test]
    fn durations_are_padded() {
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(600), "10:00");
        assert_eq!(format_duration(9), "0:09");
    }

    #[test]
    fn unknown_duration_renders_as_placeholder() {
        assert_eq!(Track::default().duration_display(), "--:--");
    }
}
