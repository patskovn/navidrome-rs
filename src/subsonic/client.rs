use anyhow::Context;
use async_trait::async_trait;
use opensubsonic::Auth;
use opensubsonic::Client;
use opensubsonic::AlbumListType;
use opensubsonic::data::AlbumId3;
use opensubsonic::data::Child;

use super::Album;
use super::AlbumSort;
use super::Library;
use super::SearchResults;
use super::Track;

/// How many albums to pull per page when walking the library.
const ALBUM_PAGE: i32 = 500;

/// [`Library`] backed by a real Subsonic/OpenSubsonic server.
pub struct OpenSubsonicLibrary {
    client: Client,
}

impl OpenSubsonicLibrary {
    pub fn new(url: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let client = Client::new(url, Auth::token(username, password))
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("connecting to {url}"))?;
        Ok(Self { client })
    }
}

fn album_from(value: AlbumId3) -> Album {
    Album {
        id: value.id,
        name: value.name,
        artist: value.artist,
        year: value.year,
        song_count: value.song_count,
    }
}

fn track_from(value: Child) -> Track {
    Track {
        id: value.id,
        title: value.title,
        artist: value.artist,
        album: value.album,
        track_no: value.track,
        duration: value.duration,
        suffix: value.suffix,
    }
}

impl From<AlbumSort> for AlbumListType {
    fn from(value: AlbumSort) -> Self {
        match value {
            AlbumSort::Alphabetical => AlbumListType::AlphabeticalByName,
            AlbumSort::Recent => AlbumListType::Newest,
            AlbumSort::Random => AlbumListType::Random,
            AlbumSort::Frequent => AlbumListType::Frequent,
        }
    }
}

#[async_trait]
impl Library for OpenSubsonicLibrary {
    async fn ping(&self) -> anyhow::Result<()> {
        self.client
            .ping()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .context("server rejected the request")
    }

    async fn albums(&self, sort: AlbumSort, offset: u32, limit: u32) -> anyhow::Result<Vec<Album>> {
        let albums = self
            .client
            .get_album_list2(
                sort.into(),
                Some(limit as i32),
                Some(offset as i32),
                None,
                None,
                None,
                None,
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .context("listing albums")?;
        Ok(albums.into_iter().map(album_from).collect())
    }

    async fn album_tracks(&self, album_id: &str) -> anyhow::Result<Vec<Track>> {
        let album = self
            .client
            .get_album(album_id)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("loading album {album_id}"))?;

        let mut tracks: Vec<Track> = album.song.into_iter().map(track_from).collect();
        tracks.sort_by_key(|t| t.track_no.unwrap_or(i32::MAX));
        Ok(tracks)
    }

    /// Subsonic has no "every song" endpoint, so "all music" is assembled by
    /// walking albums alphabetically and concatenating their tracks.
    ///
    /// `offset`/`limit` are in *albums*, not tracks: paging by track would mean
    /// re-fetching albums to find where the previous page ended. The UI treats
    /// this as an incrementally growing list, which matches how it is scrolled.
    async fn all_tracks(&self, offset: u32, limit: u32) -> anyhow::Result<Vec<Track>> {
        let albums = self
            .albums(AlbumSort::Alphabetical, offset, limit.min(ALBUM_PAGE as u32))
            .await?;

        let mut tracks = Vec::new();
        for album in albums {
            match self.album_tracks(&album.id).await {
                Ok(mut album_tracks) => tracks.append(&mut album_tracks),
                // One unreadable album should not empty the whole view.
                Err(err) => log::warn!("skipping album {}: {err:#}", album.id),
            }
        }
        Ok(tracks)
    }

    async fn search(&self, query: &str) -> anyhow::Result<SearchResults> {
        let result = self
            .client
            .search3(
                query,
                Some(0),
                None,
                Some(50),
                None,
                Some(100),
                None,
                None,
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("searching for {query:?}"))?;

        Ok(SearchResults {
            albums: result.album.into_iter().map(album_from).collect(),
            tracks: result.song.into_iter().map(track_from).collect(),
        })
    }

    fn stream_url(&self, track_id: &str) -> anyhow::Result<String> {
        // `None` for bitrate and format asks the server for the original file,
        // untranscoded — bit-perfect, and what the cache stores.
        let url = self
            .client
            .stream_url(track_id, None, None)
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("building stream URL for {track_id}"))?;
        Ok(url.to_string())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // The upstream types are deserialize-only (no `Default`), so fixtures are
    // built from the wire format. That makes these tests cover the JSON shape
    // Navidrome actually returns, not just our field mapping.

    #[test]
    fn maps_album_fields() {
        let json = serde_json::json!({
            "id": "1",
            "name": "Kind of Blue",
            "artist": "Miles Davis",
            "year": 1959,
            "songCount": 5,
            "duration": 2610,
        });
        let album = album_from(serde_json::from_value::<AlbumId3>(json).unwrap());

        assert_eq!(album.id, "1");
        assert_eq!(album.name, "Kind of Blue");
        assert_eq!(album.artist.as_deref(), Some("Miles Davis"));
        assert_eq!(album.year, Some(1959));
        assert_eq!(album.song_count, Some(5));
    }

    #[test]
    fn maps_track_fields() {
        let json = serde_json::json!({
            "id": "7",
            "isDir": false,
            "title": "So What",
            "artist": "Miles Davis",
            "album": "Kind of Blue",
            "track": 1,
            "duration": 545,
            "suffix": "flac",
        });
        let track = track_from(serde_json::from_value::<Child>(json).unwrap());

        assert_eq!(track.display(), "Miles Davis \u{2014} So What");
        assert_eq!(track.duration_display(), "9:05");
        assert_eq!(track.suffix.as_deref(), Some("flac"));
        assert_eq!(track.track_no, Some(1));
    }

    #[test]
    fn a_track_missing_optional_tags_still_maps() {
        // Navidrome omits absent tags entirely rather than sending nulls.
        let json = serde_json::json!({ "id": "9", "isDir": false, "title": "Untitled" });
        let track = track_from(serde_json::from_value::<Child>(json).unwrap());

        assert_eq!(track.display(), "Untitled");
        assert_eq!(track.duration_display(), "--:--");
        assert!(track.suffix.is_none());
    }

    #[test]
    fn sort_maps_to_api_list_type() {
        assert_eq!(
            AlbumListType::from(AlbumSort::Alphabetical),
            AlbumListType::AlphabeticalByName
        );
        assert_eq!(AlbumListType::from(AlbumSort::Recent), AlbumListType::Newest);
        assert_eq!(AlbumListType::from(AlbumSort::Random), AlbumListType::Random);
    }
}
