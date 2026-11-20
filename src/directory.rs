//! Finding stations: the radio-browser.info directory.
//!
//! radio-browser is a free, community-run catalogue of internet radio with a
//! public JSON API and no key. Its etiquette, which this follows: find the
//! current mirrors by asking `all.api.radio-browser.info` rather than hard
//! coding one, fail over between them, send a User-Agent that names the app,
//! and report a "click" when somebody actually starts a station.
//!
//! Behind [`RadioDirectory`] so reducers are tested against a fake, and this
//! module's own tests talk to a local server — never to the real directory.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde::Deserialize;

/// A station as the directory lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// radio-browser's id, used to report a click.
    pub uuid: String,
    pub name: String,
    pub stream_url: String,
    pub homepage: Option<String>,
    pub tags: Vec<String>,
    pub country: Option<String>,
    pub codec: Option<String>,
    /// kbit/s.
    pub bitrate: Option<u32>,
    pub clicks: u64,
    pub votes: u64,
}

impl Found {
    /// "MP3 128k · US · chillout, lofi" — what tells two "Lofi Radio"s apart.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match (&self.codec, self.bitrate) {
            (Some(codec), Some(br)) => parts.push(format!("{codec} {br}k")),
            (Some(codec), None) => parts.push(codec.clone()),
            (None, Some(br)) => parts.push(format!("{br}k")),
            (None, None) => {}
        }
        if let Some(country) = &self.country {
            parts.push(country.clone());
        }
        if !self.tags.is_empty() {
            parts.push(
                self.tags
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        parts.join(" · ")
    }
}

#[async_trait]
pub trait RadioDirectory: Send + Sync + 'static {
    /// Stations matching `query` by name or by tag, most listened first.
    async fn search(&self, query: &str) -> anyhow::Result<Vec<Found>>;

    /// Tells the directory a station was started, which is how it ranks them.
    async fn count_click(&self, uuid: &str) -> anyhow::Result<()>;
}

pub type SharedDirectory = Arc<dyn RadioDirectory>;

/// Results kept after merging the name and tag searches.
const LIMIT: usize = 50;

/// Asked for current mirrors, by DNS round-robin.
const DISCOVERY: &str = "https://all.api.radio-browser.info";

/// Used when discovery itself fails. Kept short: a dead entry only costs a
/// failed connection before the next is tried.
const FALLBACK: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
];

/// The wire format. Every field is defaulted: the directory is crowd-sourced,
/// and one odd record should cost that record, not the whole search.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Wire {
    stationuuid: String,
    name: String,
    url: String,
    url_resolved: String,
    homepage: String,
    tags: String,
    countrycode: String,
    codec: String,
    bitrate: u32,
    hls: u8,
    clickcount: u64,
    votes: u64,
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// A playable station from a directory record, or `None` for one this app
/// cannot use: HLS (not supported yet), no http(s) address, or no name.
fn found_from(wire: Wire) -> Option<Found> {
    if wire.hls != 0 {
        return None;
    }
    // `url_resolved` is the directory's own resolution of playlists and
    // redirects; the raw `url` is often a .pls.
    let stream_url = non_empty(wire.url_resolved).or_else(|| non_empty(wire.url))?;
    if !(stream_url.starts_with("http://") || stream_url.starts_with("https://")) {
        return None;
    }
    let name = non_empty(wire.name)?;
    Some(Found {
        uuid: wire.stationuuid,
        name,
        stream_url,
        homepage: non_empty(wire.homepage),
        tags: wire
            .tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        country: non_empty(wire.countrycode),
        codec: non_empty(wire.codec).filter(|c| c != "UNKNOWN"),
        bitrate: (wire.bitrate > 0).then_some(wire.bitrate),
        clicks: wire.clickcount,
        votes: wire.votes,
    })
}

/// How a query is put to the directory.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Nothing,
    /// One phrase, matched against names and against tags.
    NameOrTag(String),
    /// A comma-separated query: stations carrying every one of these tags.
    ///
    /// Commas, not spaces, separate them, because tags and names have spaces
    /// in them — "lofi hip hop" is one tag, "I Love Chillhop" one name.
    AllTags(Vec<String>),
}

fn plan(query: &str) -> Plan {
    let query = query.trim();
    if !query.contains(',') {
        return match query {
            "" => Plan::Nothing,
            phrase => Plan::NameOrTag(phrase.to_string()),
        };
    }
    let tags: Vec<String> = query
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect();
    // "lofi," is still a tag search, just of one tag; "," is no search.
    if tags.is_empty() {
        Plan::Nothing
    } else {
        Plan::AllTags(tags)
    }
}

/// One list from a name search and a tag search: each station once, the most
/// listened first.
///
/// Both are asked because people search both ways — "chillhop" is a name,
/// "lofi" mostly a tag — and the directory ANDs its filters rather than ORing
/// them.
fn merge(lists: impl IntoIterator<Item = Vec<Found>>) -> Vec<Found> {
    let mut seen = std::collections::HashSet::new();
    let mut all: Vec<Found> = lists
        .into_iter()
        .flatten()
        .filter(|f| seen.insert(f.uuid.clone()))
        .collect();
    all.sort_by(|a, b| b.clicks.cmp(&a.clicks).then(b.votes.cmp(&a.votes)));
    all.truncate(LIMIT);
    all
}

/// [`RadioDirectory`] backed by radio-browser.info.
pub struct RadioBrowser {
    client: reqwest::Client,
    /// Where to ask for mirrors; `None` pins the list given at construction.
    discovery: Option<String>,
    /// Mirrors, once known. Discovered on first use, not at startup, so the
    /// app does not touch the network for a feature nobody opened.
    servers: Mutex<Option<Vec<String>>>,
}

impl RadioBrowser {
    pub fn new() -> anyhow::Result<Self> {
        Self::build(Some(DISCOVERY.to_string()), None)
    }

    fn build(discovery: Option<String>, servers: Option<Vec<String>>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(crate::probe::USER_AGENT)
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            client,
            discovery,
            servers: Mutex::new(servers),
        })
    }

    async fn servers(&self) -> Vec<String> {
        if let Some(known) = self.servers.lock().expect("poisoned").clone() {
            return known;
        }
        let discovered = match &self.discovery {
            Some(discovery) => self.discover(discovery).await.unwrap_or_else(|err| {
                log::warn!("radio-browser discovery failed, using fallbacks: {err:#}");
                Vec::new()
            }),
            None => Vec::new(),
        };
        let servers = if discovered.is_empty() {
            FALLBACK.iter().map(|s| s.to_string()).collect()
        } else {
            discovered
        };
        *self.servers.lock().expect("poisoned") = Some(servers.clone());
        servers
    }

    async fn discover(&self, discovery: &str) -> anyhow::Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Server {
            name: String,
        }
        let servers: Vec<Server> = self
            .client
            .get(format!("{discovery}/json/servers"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // The list repeats entries, one per address family.
        let mut names: Vec<String> = Vec::new();
        for server in servers {
            let url = format!("https://{}", server.name);
            if !names.contains(&url) {
                names.push(url);
            }
        }
        Ok(names)
    }

    /// GETs `path` from the first mirror that answers.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<reqwest::Response> {
        let mut last = None;
        for server in self.servers().await {
            let result = self
                .client
                .get(format!("{server}{path}"))
                .query(query)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    log::debug!("radio-browser mirror {server} failed: {err}");
                    last = Some(err);
                }
            }
        }
        Err(match last {
            Some(err) => anyhow::anyhow!("radio-browser.info is not answering: {err}"),
            None => anyhow::anyhow!("no radio-browser.info mirror is known"),
        })
    }

    async fn search_by(&self, field: &str, query: &str) -> anyhow::Result<Vec<Found>> {
        let limit = LIMIT.to_string();
        let wire: Vec<Wire> = self
            .get(
                "/json/stations/search",
                &[
                    (field, query),
                    ("order", "clickcount"),
                    ("reverse", "true"),
                    ("hidebroken", "true"),
                    ("limit", &limit),
                ],
            )
            .await?
            .json()
            .await
            .context("radio-browser.info sent something unreadable")?;
        Ok(wire.into_iter().filter_map(found_from).collect())
    }
}

#[async_trait]
impl RadioDirectory for RadioBrowser {
    async fn search(&self, query: &str) -> anyhow::Result<Vec<Found>> {
        let query = match plan(query) {
            Plan::Nothing => return Ok(Vec::new()),
            // The directory's `tagList` is an AND of partial matches: every
            // tag must be on the station, and `chill` also finds `chillout`.
            Plan::AllTags(tags) => {
                let all = self.search_by("tagList", &tags.join(",")).await?;
                return Ok(merge([all]));
            }
            Plan::NameOrTag(phrase) => phrase,
        };
        let query = query.as_str();
        let (by_name, by_tag) =
            tokio::join!(self.search_by("name", query), self.search_by("tag", query));
        match (by_name, by_tag) {
            (Err(err), Err(_)) => Err(err),
            (by_name, by_tag) => Ok(merge([
                by_name.unwrap_or_default(),
                by_tag.unwrap_or_default(),
            ])),
        }
    }

    async fn count_click(&self, uuid: &str) -> anyhow::Result<()> {
        self.get(&format!("/json/url/{uuid}"), &[]).await?;
        Ok(())
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// Answers every search with `results`, recording what it was asked.
    #[derive(Default)]
    pub struct FakeDirectory {
        pub results: Vec<Found>,
        pub calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl RadioDirectory for FakeDirectory {
        async fn search(&self, query: &str) -> anyhow::Result<Vec<Found>> {
            self.calls
                .lock()
                .expect("poisoned")
                .push(format!("search {query}"));
            Ok(self.results.clone())
        }

        async fn count_click(&self, uuid: &str) -> anyhow::Result<()> {
            self.calls
                .lock()
                .expect("poisoned")
                .push(format!("click {uuid}"));
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    fn wire(uuid: &str, name: &str, clicks: u64) -> Wire {
        Wire {
            stationuuid: uuid.into(),
            name: name.into(),
            url_resolved: format!("https://{uuid}.radio.invalid/live"),
            clickcount: clicks,
            ..Default::default()
        }
    }

    /// The shape radio-browser really sends, trimmed to one record.
    #[test]
    fn a_directory_record_maps() {
        let json = serde_json::json!([{
            "stationuuid": "4260f1f5",
            "name": "Lofi 24/7",
            "url": "http://usa9.example/proxy/jamz?mp=/1",
            "url_resolved": "http://usa9.example/proxy/jamz?mp=/1",
            "homepage": "https://lofi247.example/",
            "tags": "chillout,lofi",
            "countrycode": "US",
            "codec": "MP3",
            "bitrate": 128,
            "hls": 0,
            "clickcount": 122,
            "votes": 1343,
            "lastcheckok": 1
        }]);
        let wire: Vec<Wire> = serde_json::from_value(json).unwrap();
        let found = found_from(wire.into_iter().next().unwrap()).unwrap();

        assert_eq!(found.name, "Lofi 24/7");
        assert_eq!(found.stream_url, "http://usa9.example/proxy/jamz?mp=/1");
        assert_eq!(found.tags, ["chillout", "lofi"]);
        assert_eq!(found.summary(), "MP3 128k · US · chillout, lofi");
        assert_eq!(found.votes, 1343);
    }

    #[test]
    fn unusable_records_are_skipped() {
        let hls = Wire {
            hls: 1,
            ..wire("a", "HLS FM", 1)
        };
        let nameless = wire("b", "  ", 1);
        let ftp = Wire {
            url_resolved: "ftp://x.example/".into(),
            ..wire("c", "FTP FM", 1)
        };
        assert!(found_from(hls).is_none());
        assert!(found_from(nameless).is_none());
        assert!(found_from(ftp).is_none());
    }

    #[test]
    fn the_raw_url_is_used_when_nothing_was_resolved() {
        let w = Wire {
            url_resolved: String::new(),
            url: "https://raw.radio.invalid/".into(),
            ..wire("a", "Raw", 1)
        };
        assert_eq!(
            found_from(w).unwrap().stream_url,
            "https://raw.radio.invalid/"
        );
    }

    #[test]
    fn name_and_tag_hits_merge_once_each_most_listened_first() {
        let f = |uuid: &str, clicks| found_from(wire(uuid, uuid, clicks)).unwrap();
        let merged = merge([vec![f("a", 5), f("b", 50)], vec![f("b", 50), f("c", 20)]]);
        let order: Vec<_> = merged.iter().map(|f| f.uuid.as_str()).collect();
        assert_eq!(order, ["b", "c", "a"]);
    }

    /// A stand-in for a mirror: answers searches by which filter was used.
    async fn mirror() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = vec![0u8; 4096];
                    let n = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..n]).to_string();
                    let target = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    // Decoded into exact parameters: substring matching
                    // would let `name=lofi` answer for `name=lofi hip hop`.
                    let url = reqwest::Url::parse(&format!("http://mirror{target}")).unwrap();
                    let param = |key: &str| {
                        url.query_pairs()
                            .find(|(k, _)| k == key)
                            .map(|(_, v)| v.into_owned())
                    };
                    let (name, tag, tag_list) = (param("name"), param("tag"), param("tagList"));
                    let body = match (name.as_deref(), tag.as_deref(), tag_list.as_deref()) {
                        (_, _, Some("lofi,chill")) => {
                            r#"[{"stationuuid":"both","name":"Lofi and Chill","url_resolved":"https://both.radio.invalid/","clickcount":5}]"#
                        }
                        (_, Some("lofi hip hop"), _) => {
                            r#"[{"stationuuid":"hh","name":"Hip Hop Lofi","url_resolved":"https://hh.radio.invalid/","clickcount":3}]"#
                        }
                        (Some("lofi"), _, _) => {
                            r#"[{"stationuuid":"n1","name":"Lofi Name FM","url_resolved":"https://n1.radio.invalid/","clickcount":10}]"#
                        }
                        (_, Some("lofi"), _) => {
                            r#"[{"stationuuid":"t1","name":"Tagged Lofi","url_resolved":"https://t1.radio.invalid/","clickcount":99},{"stationuuid":"n1","name":"Lofi Name FM","url_resolved":"https://n1.radio.invalid/","clickcount":10}]"#
                        }
                        _ if url.path().starts_with("/json/url/") => r#"{"ok":true}"#,
                        _ => "[]",
                    };
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        base
    }

    /// An address nothing listens on: bound, then released.
    async fn dead_mirror() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        base
    }

    #[tokio::test]
    async fn search_asks_by_name_and_by_tag_and_merges() {
        let directory = RadioBrowser::build(None, Some(vec![mirror().await])).unwrap();
        let found = directory.search("lofi").await.unwrap();
        let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Tagged Lofi", "Lofi Name FM"]);
    }

    #[test]
    fn commas_make_a_query_a_list_of_tags() {
        assert_eq!(plan("  "), Plan::Nothing);
        assert_eq!(plan("lofi"), Plan::NameOrTag("lofi".into()));
        // Spaces alone never split: they are part of tags and names.
        assert_eq!(plan("lofi hip hop"), Plan::NameOrTag("lofi hip hop".into()));
        assert_eq!(
            plan("lofi, chill"),
            Plan::AllTags(vec!["lofi".into(), "chill".into()])
        );
        assert_eq!(
            plan(" lofi hip hop ,chill,, "),
            Plan::AllTags(vec!["lofi hip hop".into(), "chill".into()])
        );
        assert_eq!(plan("lofi,"), Plan::AllTags(vec!["lofi".into()]));
        assert_eq!(plan(" , ,"), Plan::Nothing);
    }

    /// Several tags go out as one `tagList`, not as a name or a single tag.
    #[tokio::test]
    async fn several_tags_ask_for_stations_with_all_of_them() {
        let directory = RadioBrowser::build(None, Some(vec![mirror().await])).unwrap();
        let found = directory.search("lofi, chill").await.unwrap();
        let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Lofi and Chill"]);
    }

    /// A phrase with spaces is one tag (or name), sent whole.
    #[tokio::test]
    async fn a_phrase_with_spaces_is_one_tag() {
        let directory = RadioBrowser::build(None, Some(vec![mirror().await])).unwrap();
        let found = directory.search("lofi hip hop").await.unwrap();
        let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Hip Hop Lofi"]);
    }

    #[tokio::test]
    async fn a_dead_mirror_fails_over_to_the_next() {
        let directory =
            RadioBrowser::build(None, Some(vec![dead_mirror().await, mirror().await])).unwrap();
        assert_eq!(directory.search("lofi").await.unwrap().len(), 2);
        directory.count_click("t1").await.unwrap();
    }

    #[tokio::test]
    async fn no_mirror_answering_is_an_error_not_an_empty_result() {
        let directory = RadioBrowser::build(None, Some(vec![dead_mirror().await])).unwrap();
        let err = format!("{:#}", directory.search("lofi").await.unwrap_err());
        assert!(err.contains("not answering"), "{err}");
    }

    /// The real directory, discovery included. Opt-in, because it reaches
    /// radio-browser.info: `cargo test real_directory -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "reaches radio-browser.info"]
    async fn real_directory() {
        let directory = RadioBrowser::new().unwrap();
        for query in ["lofi", "synthwave", "lofi, chill", "lofi, synthwave"] {
            let found = directory.search(query).await.unwrap();
            println!("DIRECTORY {query}: {} stations", found.len());
            for f in found.iter().take(5) {
                println!("  {:<40} {}  <{}>", f.name, f.summary(), f.stream_url);
            }
            assert!(!found.is_empty(), "nothing for {query}");
        }
        println!("MIRRORS {:?}", directory.servers.lock().unwrap());
    }

    #[tokio::test]
    async fn an_empty_query_asks_nothing() {
        let directory = RadioBrowser::build(None, Some(vec![dead_mirror().await])).unwrap();
        assert!(directory.search("   ").await.unwrap().is_empty());
    }
}
