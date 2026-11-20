//! Checking that a URL really is a radio stream before trusting the player
//! with it.
//!
//! The add-station form's preview runs through here first. A bare "it failed
//! to decode" from the audio thread says nothing useful about a URL somebody
//! just pasted; this can say *why* — a 404, a web page, a playlist, an HLS
//! stream — and read the station's own name and bitrate from its ICY headers
//! while it is at it.
//!
//! Lives behind [`StreamProbe`] so reducers can be tested against a fake,
//! the same reason the server sits behind `Library`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;

/// What a successful probe learned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProbeInfo {
    /// Where the audio actually is. Differs from what was typed when that was
    /// a `.pls`/`.m3u` playlist pointing at the stream.
    pub stream_url: String,
    pub content_type: String,
    /// `icy-name`: what the station calls itself.
    pub name: Option<String>,
    /// `icy-br`, in kbit/s.
    pub bitrate: Option<u32>,
    pub genre: Option<String>,
}

#[async_trait]
pub trait StreamProbe: Send + Sync + 'static {
    async fn probe(&self, url: &str) -> anyhow::Result<ProbeInfo>;
}

pub type SharedProbe = Arc<dyn StreamProbe>;

/// What this app calls itself to a station. Shared with the player, so the
/// probe and the playback that follows it look the same to the server — some
/// stations refuse a client that sends no User-Agent at all.
pub const USER_AGENT: &str = concat!("navidrome-rs/", env!("CARGO_PKG_VERSION"));

/// Playlists worth reading are a few lines long. Anything bigger is not one,
/// and an audio stream mislabelled as a playlist would otherwise be read
/// forever.
const MAX_PLAYLIST_BYTES: usize = 64 * 1024;

/// Playlists that point at playlists are legal, but a loop is not worth
/// following far.
const MAX_HOPS: usize = 3;

/// [`StreamProbe`] over real HTTP.
pub struct HttpProbe {
    client: reqwest::Client,
}

impl HttpProbe {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // Covers headers, and a playlist body; an audio body is never read.
            .timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .context("building the HTTP client")?;
        Ok(Self { client })
    }
}

/// What a response turned out to be.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Audio,
    Playlist,
    Hls,
    Other,
}

/// Decides from the content type, falling back on the path's extension for
/// servers that label everything `text/plain` or `application/octet-stream`.
fn classify(content_type: &str, path: &str) -> Kind {
    let path = path.to_ascii_lowercase();
    // HLS first: `application/x-mpegurl` is used for both, and only the
    // extension tells an HLS index from a plain M3U.
    if content_type == "application/vnd.apple.mpegurl" || path.ends_with(".m3u8") {
        return Kind::Hls;
    }
    // Before audio: `audio/x-mpegurl` starts with `audio/` but is a list.
    let playlist_type = matches!(
        content_type,
        "audio/x-mpegurl"
            | "audio/mpegurl"
            | "application/x-mpegurl"
            | "audio/x-scpls"
            | "application/pls+xml"
    );
    if playlist_type || path.ends_with(".pls") || path.ends_with(".m3u") {
        return Kind::Playlist;
    }
    if content_type.starts_with("audio/") || content_type == "application/ogg" {
        return Kind::Audio;
    }
    Kind::Other
}

/// The first stream address in a `.pls` or `.m3u` body.
///
/// Both formats reduce to "a line that is an http(s) URL", once the PLS
/// `FileN=` prefix is taken off; comments and metadata lines never are.
fn first_stream_url(playlist: &str) -> Option<String> {
    playlist.lines().find_map(|line| {
        let line = line.trim();
        let value = match line.split_once('=') {
            Some((key, value)) if key.to_ascii_lowercase().starts_with("file") => value.trim(),
            _ => line,
        };
        (value.starts_with("http://") || value.starts_with("https://")).then(|| value.to_string())
    })
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    let value = response.headers().get(name)?.to_str().ok()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[async_trait]
impl StreamProbe for HttpProbe {
    async fn probe(&self, url: &str) -> anyhow::Result<ProbeInfo> {
        let mut url = url.trim().to_string();
        for _ in 0..MAX_HOPS {
            let parsed = reqwest::Url::parse(&url).context("that is not a URL")?;
            if !matches!(parsed.scheme(), "http" | "https") {
                anyhow::bail!("only http:// and https:// streams can be played");
            }
            let path = parsed.path().to_string();

            // Asking for no interleaved metadata keeps the body pure audio,
            // which is what the player will also ask for.
            let mut response = self
                .client
                .get(parsed)
                .header("Icy-MetaData", "0")
                .send()
                .await
                .map_err(|err| anyhow::anyhow!("could not reach it: {err}"))?;
            let status = response.status();
            if !status.is_success() {
                anyhow::bail!("the server answered {status}");
            }
            let content_type = header(&response, "content-type")
                .map(|v| {
                    v.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();

            match classify(&content_type, &path) {
                // Returned without reading the body: an endless stream has no
                // end to wait for, and dropping the response hangs up.
                Kind::Audio => {
                    return Ok(ProbeInfo {
                        stream_url: url,
                        name: header(&response, "icy-name"),
                        bitrate: header(&response, "icy-br")
                            .and_then(|br| br.split(',').next()?.trim().parse().ok()),
                        genre: header(&response, "icy-genre"),
                        content_type,
                    });
                }
                Kind::Playlist => {
                    let mut body = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        body.extend_from_slice(&chunk);
                        if body.len() > MAX_PLAYLIST_BYTES {
                            anyhow::bail!("that playlist is too large to be one");
                        }
                    }
                    let text = String::from_utf8_lossy(&body);
                    url = first_stream_url(&text).context("that playlist lists no stream")?;
                }
                Kind::Hls => anyhow::bail!("HLS (.m3u8) streams are not supported yet"),
                Kind::Other if content_type.is_empty() => {
                    anyhow::bail!("the server did not say what it sent")
                }
                Kind::Other => anyhow::bail!("that is not audio: the server sent {content_type}"),
            }
        }
        anyhow::bail!("too many playlists pointing at playlists")
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Answers every probe with `result`, recording what it was asked.
    pub struct FakeProbe {
        pub result: Result<ProbeInfo, String>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeProbe {
        pub fn ok(info: ProbeInfo) -> Self {
            Self {
                result: Ok(info),
                calls: Mutex::default(),
            }
        }

        pub fn failing(message: &str) -> Self {
            Self {
                result: Err(message.to_string()),
                calls: Mutex::default(),
            }
        }
    }

    #[async_trait]
    impl StreamProbe for FakeProbe {
        async fn probe(&self, url: &str) -> anyhow::Result<ProbeInfo> {
            self.calls.lock().expect("poisoned").push(url.to_string());
            self.result
                .clone()
                .map_err(|message| anyhow::anyhow!(message))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn content_types_are_told_apart() {
        assert_eq!(classify("audio/mpeg", "/live"), Kind::Audio);
        assert_eq!(classify("audio/aac", "/lofi_normal"), Kind::Audio);
        assert_eq!(classify("application/ogg", "/ogg"), Kind::Audio);
        assert_eq!(classify("audio/x-scpls", "/listen"), Kind::Playlist);
        assert_eq!(classify("audio/x-mpegurl", "/listen"), Kind::Playlist);
        assert_eq!(classify("text/plain", "/station.pls"), Kind::Playlist);
        assert_eq!(classify("application/x-mpegurl", "/live.m3u8"), Kind::Hls);
        assert_eq!(classify("application/vnd.apple.mpegurl", "/x"), Kind::Hls);
        assert_eq!(classify("text/html", "/"), Kind::Other);
    }

    #[test]
    fn pls_and_m3u_both_yield_their_first_stream() {
        let pls = "[playlist]\nNumberOfEntries=2\nFile1=https://a.example/one\nTitle1=One\nFile2=https://b.example/two\n";
        assert_eq!(
            first_stream_url(pls).as_deref(),
            Some("https://a.example/one")
        );

        let m3u = "#EXTM3U\n#EXTINF:-1,Chill\nhttp://c.example/chill.mp3\n";
        assert_eq!(
            first_stream_url(m3u).as_deref(),
            Some("http://c.example/chill.mp3")
        );

        assert_eq!(first_stream_url("#EXTM3U\n# nothing here\n"), None);
    }

    /// A one-file HTTP server on localhost, answering by path. Probing real
    /// stations from a test would make it depend on strangers' servers.
    async fn serve() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stream_url = format!("{base}/stream");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let stream_url = stream_url.clone();
                tokio::spawn(async move {
                    let mut request = vec![0u8; 2048];
                    let n = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..n]);
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let reply = match path.as_str() {
                        // No length and no end, like a real station: the probe
                        // must not wait for the body.
                        "/stream" => "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nicy-name: Test FM\r\nicy-br: 192\r\nicy-genre: lofi\r\n\r\n".to_string(),
                        "/station.pls" => {
                            let body = format!("[playlist]\nFile1={stream_url}\n");
                            format!("HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                        }
                        "/page" => "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi".to_string(),
                        "/live.m3u8" => "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                        _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                    };
                    let _ = socket.write_all(reply.as_bytes()).await;
                    if path == "/stream" {
                        // Keep "streaming" until the client hangs up.
                        while socket.write_all(&[0xFF; 1024]).await.is_ok() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                });
            }
        });
        base
    }

    #[tokio::test]
    async fn a_live_stream_is_accepted_without_waiting_for_its_end() {
        let base = serve().await;
        let info = tokio::time::timeout(
            Duration::from_secs(5),
            HttpProbe::new().unwrap().probe(&format!("{base}/stream")),
        )
        .await
        .expect("the probe must not wait for an endless body")
        .unwrap();

        assert_eq!(info.stream_url, format!("{base}/stream"));
        assert_eq!(info.content_type, "audio/mpeg");
        assert_eq!(info.name.as_deref(), Some("Test FM"));
        assert_eq!(info.bitrate, Some(192));
        assert_eq!(info.genre.as_deref(), Some("lofi"));
    }

    #[tokio::test]
    async fn a_playlist_is_followed_to_its_stream() {
        let base = serve().await;
        let info = HttpProbe::new()
            .unwrap()
            .probe(&format!("{base}/station.pls"))
            .await
            .unwrap();
        assert_eq!(info.stream_url, format!("{base}/stream"));
        assert_eq!(info.name.as_deref(), Some("Test FM"));
    }

    #[tokio::test]
    async fn failures_say_why() {
        let base = serve().await;
        let probe = HttpProbe::new().unwrap();
        let why = |path: &'static str| {
            let url = format!("{base}{path}");
            let probe = &probe;
            async move { format!("{:#}", probe.probe(&url).await.unwrap_err()) }
        };

        assert!(why("/missing").await.contains("404"));
        assert!(why("/page").await.contains("text/html"));
        assert!(why("/live.m3u8").await.contains("HLS"));
        let not_a_url = format!("{:#}", probe.probe("chillhop radio").await.unwrap_err());
        assert!(not_a_url.contains("not a URL"), "{not_a_url}");
        let ftp = format!("{:#}", probe.probe("ftp://x.example/a").await.unwrap_err());
        assert!(ftp.contains("http"), "{ftp}");
    }
}
