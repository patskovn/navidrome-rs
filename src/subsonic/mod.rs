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

/// An album artist, as listed by `getArtists`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub album_count: Option<i64>,
    /// Id to fetch artwork with; not the image itself.
    pub cover_art: Option<String>,
}

/// An internet radio station, as the server stores it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub stream_url: String,
    pub home_page_url: Option<String>,
}

impl Station {
    /// The station as something the player can queue. It plays from its own
    /// URL, live, rather than through the server's `stream` endpoint.
    pub fn as_track(&self) -> Track {
        Track {
            id: format!("radio:{}", self.id),
            title: self.name.clone(),
            artist: Some("Internet radio".into()),
            live_url: Some(self.stream_url.clone()),
            ..Default::default()
        }
    }
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
    /// Id to fetch artwork with; not the image itself. Usually the album's.
    pub cover_art: Option<String>,
    /// Set for a live source — a radio station — which plays from this URL
    /// with no length, no seeking and no cache, instead of through the
    /// server's `stream` endpoint.
    pub live_url: Option<String>,
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
        if self.live_url.is_some() {
            return "LIVE".to_string();
        }
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

    /// Every album artist on the server, in the server's order. Subsonic
    /// returns the whole roster in one response, so there is no paging.
    async fn artists(&self) -> anyhow::Result<Vec<Artist>>;

    /// One artist's albums, oldest first.
    async fn artist_albums(&self, artist_id: &str) -> anyhow::Result<Vec<Album>>;

    /// The internet radio stations configured on the server.
    async fn radios(&self) -> anyhow::Result<Vec<Station>>;

    /// Adds a station. Navidrome only lets admins do this.
    async fn create_radio(&self, name: &str, stream_url: &str) -> anyhow::Result<()>;

    async fn delete_radio(&self, id: &str) -> anyhow::Result<()>;

    /// A page of "all music". Subsonic has no true "all songs" endpoint, so
    /// this is served by walking albums — see the implementation.
    async fn all_tracks(&self, offset: u32, limit: u32) -> anyhow::Result<Vec<Track>>;

    async fn search(&self, query: &str) -> anyhow::Result<SearchResults>;

    /// Encoded artwork bytes for a `cover_art` id.
    async fn cover_art(&self, cover_art_id: &str) -> anyhow::Result<Vec<u8>>;

    /// A fully authenticated URL the audio layer can stream from directly.
    fn stream_url(&self, track_id: &str) -> anyhow::Result<String>;
}

/// Shared handle used by reducers.
pub type SharedLibrary = Arc<dyn Library>;

/// A [`Library`] whose backing client can be replaced at runtime.
///
/// Reducers are values built once, so the client they hold is fixed for the
/// life of the store — but signing in has to install a real client where a
/// signed-out stub was. This indirection is the join between the two. It lives
/// in the `Feature`, never in `State`, so its interior mutability cannot break
/// `TestStore` comparisons.
pub struct SwappableLibrary {
    inner: std::sync::RwLock<SharedLibrary>,
}

impl SwappableLibrary {
    pub fn new(initial: SharedLibrary) -> Self {
        Self {
            inner: std::sync::RwLock::new(initial),
        }
    }

    /// Installs a new client. Takes effect on the next call.
    pub fn set(&self, library: SharedLibrary) {
        *self.inner.write().expect("library lock poisoned") = library;
    }

    /// Clones the current client out. The guard is released before any await,
    /// which a std lock requires.
    fn current(&self) -> SharedLibrary {
        self.inner.read().expect("library lock poisoned").clone()
    }
}

#[async_trait]
impl Library for SwappableLibrary {
    async fn ping(&self) -> anyhow::Result<()> {
        self.current().ping().await
    }

    async fn albums(&self, sort: AlbumSort, offset: u32, limit: u32) -> anyhow::Result<Vec<Album>> {
        self.current().albums(sort, offset, limit).await
    }

    async fn album_tracks(&self, album_id: &str) -> anyhow::Result<Vec<Track>> {
        let library = self.current();
        let id = album_id.to_string();
        library.album_tracks(&id).await
    }

    async fn artists(&self) -> anyhow::Result<Vec<Artist>> {
        self.current().artists().await
    }

    async fn artist_albums(&self, artist_id: &str) -> anyhow::Result<Vec<Album>> {
        let library = self.current();
        let id = artist_id.to_string();
        library.artist_albums(&id).await
    }

    async fn radios(&self) -> anyhow::Result<Vec<Station>> {
        self.current().radios().await
    }

    async fn create_radio(&self, name: &str, stream_url: &str) -> anyhow::Result<()> {
        let library = self.current();
        let (name, stream_url) = (name.to_string(), stream_url.to_string());
        library.create_radio(&name, &stream_url).await
    }

    async fn delete_radio(&self, id: &str) -> anyhow::Result<()> {
        let library = self.current();
        let id = id.to_string();
        library.delete_radio(&id).await
    }

    async fn all_tracks(&self, offset: u32, limit: u32) -> anyhow::Result<Vec<Track>> {
        self.current().all_tracks(offset, limit).await
    }

    async fn search(&self, query: &str) -> anyhow::Result<SearchResults> {
        let library = self.current();
        let query = query.to_string();
        library.search(&query).await
    }

    async fn cover_art(&self, cover_art_id: &str) -> anyhow::Result<Vec<u8>> {
        let library = self.current();
        let id = cover_art_id.to_string();
        library.cover_art(&id).await
    }

    fn stream_url(&self, track_id: &str) -> anyhow::Result<String> {
        self.current().stream_url(track_id)
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// In-memory [`Library`] for reducer tests.
    #[derive(Default)]
    pub struct FakeLibrary {
        /// Mutable, so creating and deleting can be observed by the next list.
        pub stations: Mutex<Vec<Station>>,
        pub artists: Vec<Artist>,
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

        async fn artists(&self) -> anyhow::Result<Vec<Artist>> {
            self.record("artists");
            Ok(self.artists.clone())
        }

        async fn artist_albums(&self, _artist_id: &str) -> anyhow::Result<Vec<Album>> {
            self.record("artist_albums");
            Ok(self.albums.clone())
        }

        async fn radios(&self) -> anyhow::Result<Vec<Station>> {
            self.record("radios");
            Ok(self.stations.lock().expect("poisoned").clone())
        }

        async fn create_radio(&self, name: &str, stream_url: &str) -> anyhow::Result<()> {
            self.record(&format!("create_radio {name} {stream_url}"));
            let mut stations = self.stations.lock().expect("poisoned");
            let id = format!("r{}", stations.len() + 1);
            stations.push(Station {
                id,
                name: name.into(),
                stream_url: stream_url.into(),
                home_page_url: None,
            });
            Ok(())
        }

        async fn delete_radio(&self, id: &str) -> anyhow::Result<()> {
            self.record(&format!("delete_radio {id}"));
            self.stations
                .lock()
                .expect("poisoned")
                .retain(|s| s.id != id);
            Ok(())
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

        async fn cover_art(&self, _cover_art_id: &str) -> anyhow::Result<Vec<u8>> {
            self.record("cover_art");
            anyhow::bail!("no artwork in the fake library")
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
    fn a_station_plays_live_from_its_own_url() {
        let track = Station {
            id: "7".into(),
            name: "Chill".into(),
            stream_url: "https://radio.example/chill.mp3".into(),
            home_page_url: None,
        }
        .as_track();
        assert_eq!(
            track.live_url.as_deref(),
            Some("https://radio.example/chill.mp3")
        );
        assert_eq!(track.duration_display(), "LIVE");
        assert_eq!(track.title, "Chill");
    }

    #[test]
    fn unknown_duration_renders_as_placeholder() {
        assert_eq!(Track::default().duration_display(), "--:--");
    }
}
