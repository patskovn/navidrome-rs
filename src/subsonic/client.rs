use anyhow::Context;
use async_trait::async_trait;
use opensubsonic::AlbumListType;
use opensubsonic::Auth;
use opensubsonic::Client;
use opensubsonic::data::AlbumId3;
use opensubsonic::data::ArtistId3;
use opensubsonic::data::Child;
use opensubsonic::data::InternetRadioStation;

use super::Album;
use super::AlbumSort;
use super::Artist;
use super::Library;
use super::SearchResults;
use super::Station;
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
        duration: value.duration,
        genre: value.genre,
        play_count: value.play_count,
        cover_art: value.cover_art,
    }
}

fn artist_from(value: ArtistId3) -> Artist {
    Artist {
        id: value.id,
        name: value.name,
        album_count: value.album_count,
        cover_art: value.cover_art,
    }
}

fn station_from(value: InternetRadioStation) -> Station {
    Station {
        id: value.id,
        name: value.name,
        stream_url: value.stream_url,
        home_page_url: value.home_page_url,
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
        cover_art: value.cover_art,
        live_url: None,
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

    async fn artists(&self) -> anyhow::Result<Vec<Artist>> {
        let artists = self
            .client
            .get_artists(None)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .context("listing artists")?;
        // The server groups artists under index letters, already sorted with
        // its ignored articles ("The", "A") applied. Flattening keeps that
        // order rather than re-sorting on the raw name.
        Ok(artists
            .index
            .into_iter()
            .flat_map(|index| index.artist)
            .map(artist_from)
            .collect())
    }

    async fn artist_albums(&self, artist_id: &str) -> anyhow::Result<Vec<Album>> {
        let artist = self
            .client
            .get_artist(artist_id)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("loading artist {artist_id}"))?;
        let mut albums: Vec<Album> = artist.album.into_iter().map(album_from).collect();
        // A discography reads in release order; undated albums go last.
        albums.sort_by_key(|a| a.year.unwrap_or(i32::MAX));
        Ok(albums)
    }

    async fn radios(&self) -> anyhow::Result<Vec<Station>> {
        let stations = self
            .client
            .get_internet_radio_stations()
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .context("listing radio stations")?;
        Ok(stations.into_iter().map(station_from).collect())
    }

    async fn create_radio(&self, name: &str, stream_url: &str) -> anyhow::Result<()> {
        self.client
            .create_internet_radio_station(stream_url, name, None)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("adding station {name:?}"))
    }

    async fn delete_radio(&self, id: &str) -> anyhow::Result<()> {
        self.client
            .delete_internet_radio_station(id)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("deleting station {id}"))
    }

    /// Subsonic has no "every song" endpoint, so "all music" is assembled by
    /// walking albums alphabetically and concatenating their tracks.
    ///
    /// `offset`/`limit` are in *albums*, not tracks: paging by track would mean
    /// re-fetching albums to find where the previous page ended. The UI treats
    /// this as an incrementally growing list, which matches how it is scrolled.
    async fn all_tracks(&self, offset: u32, limit: u32) -> anyhow::Result<Vec<Track>> {
        let albums = self
            .albums(
                AlbumSort::Alphabetical,
                offset,
                limit.min(ALBUM_PAGE as u32),
            )
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
            .search3(query, Some(0), None, Some(50), None, Some(100), None, None)
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("searching for {query:?}"))?;

        Ok(SearchResults {
            albums: result.album.into_iter().map(album_from).collect(),
            tracks: result.song.into_iter().map(track_from).collect(),
        })
    }

    async fn cover_art(&self, cover_art_id: &str) -> anyhow::Result<Vec<u8>> {
        // 256px is plenty: the art is downscaled to a character grid anyway.
        let bytes = self
            .client
            .get_cover_art(cover_art_id, Some(256))
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("fetching artwork {cover_art_id}"))?;
        Ok(bytes.to_vec())
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
    fn maps_artist_fields() {
        let json = serde_json::json!({
            "id": "ar-1",
            "name": "Miles Davis",
            "albumCount": 12,
            "coverArt": "ar-1",
        });
        let artist = artist_from(serde_json::from_value::<ArtistId3>(json).unwrap());

        assert_eq!(artist.id, "ar-1");
        assert_eq!(artist.name, "Miles Davis");
        assert_eq!(artist.album_count, Some(12));
        assert_eq!(artist.cover_art.as_deref(), Some("ar-1"));
    }

    #[test]
    fn maps_station_fields() {
        let json = serde_json::json!({
            "id": "1",
            "name": "I Love Chillhop",
            "streamUrl": "https://ilm.example/chillhop-192",
            "homePageUrl": "https://ilm.example",
        });
        let station = station_from(serde_json::from_value::<InternetRadioStation>(json).unwrap());

        assert_eq!(station.id, "1");
        assert_eq!(station.name, "I Love Chillhop");
        assert_eq!(station.stream_url, "https://ilm.example/chillhop-192");
        assert_eq!(
            station.home_page_url.as_deref(),
            Some("https://ilm.example")
        );
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
            "coverArt": "al-7",
        });
        let track = track_from(serde_json::from_value::<Child>(json).unwrap());

        assert_eq!(track.display(), "Miles Davis \u{2014} So What");
        assert_eq!(track.duration_display(), "9:05");
        assert_eq!(track.suffix.as_deref(), Some("flac"));
        assert_eq!(track.track_no, Some(1));
        assert_eq!(track.cover_art.as_deref(), Some("al-7"));
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
        assert_eq!(
            AlbumListType::from(AlbumSort::Recent),
            AlbumListType::Newest
        );
        assert_eq!(
            AlbumListType::from(AlbumSort::Random),
            AlbumListType::Random
        );
    }
}
