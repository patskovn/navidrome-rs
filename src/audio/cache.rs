//! Persistent, size-bounded cache of original track files.
//!
//! The cache doubles as `stream-download`'s storage backend, which is what lets
//! a track be played *while* it downloads and still end up cached — one pass,
//! one download. A partially written track lives at `<id>.<ext>.part` and is
//! promoted to `<id>.<ext>` only once the full content length has landed, so an
//! interrupted download can never be mistaken for a complete file.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stream_download::storage::StorageProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    file: String,
    size: u64,
    last_used: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheIndex {
    entries: HashMap<String, CacheEntry>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An LRU cache of complete track files, bounded by total bytes on disk.
#[derive(Debug)]
pub struct TrackCache {
    dir: PathBuf,
    budget: u64,
    index: Mutex<CacheIndex>,
}

impl TrackCache {
    pub fn new(dir: impl Into<PathBuf>, budget: u64) -> anyhow::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let index = Self::load_index(&dir).unwrap_or_default();
        let cache = Self {
            dir,
            budget,
            index: Mutex::new(index),
        };
        cache.reconcile()?;
        Ok(cache)
    }

    fn index_path(dir: &Path) -> PathBuf {
        dir.join("index.json")
    }

    fn load_index(dir: &Path) -> Option<CacheIndex> {
        let bytes = std::fs::read(Self::index_path(dir)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn persist_index(&self, index: &CacheIndex) {
        if let Ok(json) = serde_json::to_vec(index)
            && let Err(err) = std::fs::write(Self::index_path(&self.dir), json)
        {
            log::warn!("could not persist cache index: {err}");
        }
    }

    /// Drops index entries whose files have vanished, and deletes stray
    /// `.part` files left behind by an interrupted run.
    fn reconcile(&self) -> anyhow::Result<()> {
        let mut index = self.index.lock().expect("cache index poisoned");
        index
            .entries
            .retain(|_, entry| self.dir.join(&entry.file).exists());

        if let Ok(read_dir) = std::fs::read_dir(&self.dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "part") {
                    log::debug!("removing stale partial download {}", path.display());
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        self.persist_index(&index);
        Ok(())
    }

    fn file_name(track_id: &str, suffix: Option<&str>) -> String {
        // Track ids come from the server and land in a path, so keep only
        // characters that are unambiguously safe as a file name.
        let safe: String = track_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        match suffix {
            Some(ext) if !ext.is_empty() => format!("{safe}.{ext}"),
            _ => safe,
        }
    }

    /// Path a completed track would occupy.
    pub fn path_for(&self, track_id: &str, suffix: Option<&str>) -> PathBuf {
        self.dir.join(Self::file_name(track_id, suffix))
    }

    fn partial_path_for(&self, track_id: &str, suffix: Option<&str>) -> PathBuf {
        self.dir
            .join(format!("{}.part", Self::file_name(track_id, suffix)))
    }

    /// Returns the cached file if this track is fully downloaded, marking it as
    /// recently used. `None` means "stream it".
    pub fn lookup(&self, track_id: &str, suffix: Option<&str>) -> Option<PathBuf> {
        let path = self.path_for(track_id, suffix);
        if !path.exists() {
            return None;
        }
        let mut index = self.index.lock().expect("cache index poisoned");
        if let Some(entry) = index.entries.get_mut(track_id) {
            entry.last_used = now_secs();
            let snapshot = CacheIndex {
                entries: index.entries.clone(),
            };
            drop(index);
            self.persist_index(&snapshot);
        }
        Some(path)
    }

    /// Records a newly completed download and evicts until back under budget.
    pub fn insert(&self, track_id: &str, path: &Path) {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            return;
        };

        {
            let mut index = self.index.lock().expect("cache index poisoned");
            index.entries.insert(
                track_id.to_string(),
                CacheEntry {
                    file: file.to_string(),
                    size,
                    last_used: now_secs(),
                },
            );
            // Persist here rather than relying on `enforce_budget`, which
            // returns early while under budget — the common case, and the one
            // where the entry would otherwise be forgotten on restart.
            let snapshot = CacheIndex {
                entries: index.entries.clone(),
            };
            drop(index);
            self.persist_index(&snapshot);
        }
        if let Err(err) = self.enforce_budget() {
            log::warn!("cache eviction failed: {err:#}");
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.index
            .lock()
            .expect("cache index poisoned")
            .entries
            .values()
            .map(|e| e.size)
            .sum()
    }

    /// Evicts least-recently-used entries until the cache fits its budget.
    /// Returns how many bytes were freed.
    pub fn enforce_budget(&self) -> anyhow::Result<u64> {
        let mut index = self.index.lock().expect("cache index poisoned");
        let mut total: u64 = index.entries.values().map(|e| e.size).sum();
        if total <= self.budget {
            return Ok(0);
        }

        let mut by_age: Vec<(String, CacheEntry)> = index
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        by_age.sort_by_key(|(_, entry)| entry.last_used);

        let mut freed = 0u64;
        for (key, entry) in by_age {
            if total <= self.budget {
                break;
            }
            let path = self.dir.join(&entry.file);
            match std::fs::remove_file(&path) {
                Ok(()) | Err(_) => {
                    // Either way the entry is gone as far as we're concerned.
                    index.entries.remove(&key);
                    total = total.saturating_sub(entry.size);
                    freed += entry.size;
                    log::debug!("evicted {} ({} bytes)", entry.file, entry.size);
                }
            }
        }

        let snapshot = CacheIndex {
            entries: index.entries.clone(),
        };
        drop(index);
        self.persist_index(&snapshot);
        Ok(freed)
    }

    /// A `stream-download` storage backend that writes straight into the cache.
    pub fn storage_provider(
        self: &Arc<Self>,
        track_id: &str,
        suffix: Option<&str>,
    ) -> CacheStorageProvider {
        CacheStorageProvider {
            cache: self.clone(),
            track_id: track_id.to_string(),
            partial: self.partial_path_for(track_id, suffix),
            final_path: self.path_for(track_id, suffix),
        }
    }
}

/// Storage backend handing `stream-download` a file inside the cache.
#[derive(Debug)]
pub struct CacheStorageProvider {
    cache: Arc<TrackCache>,
    track_id: String,
    partial: PathBuf,
    final_path: PathBuf,
}

impl StorageProvider for CacheStorageProvider {
    type Reader = File;
    type Writer = CacheWriter;

    fn into_reader_writer(
        self,
        content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)> {
        // One file, two handles: the reader trails the writer as bytes land.
        let writer = File::create(&self.partial)?;
        let reader = File::open(&self.partial)?;

        Ok((
            reader,
            CacheWriter {
                file: writer,
                cache: self.cache,
                track_id: self.track_id,
                partial: self.partial,
                final_path: self.final_path,
                content_length,
                promoted: false,
            },
        ))
    }
}

/// Write half of [`CacheStorageProvider`]. Promotes the partial file to its
/// final name once the whole track has been written.
#[derive(Debug)]
pub struct CacheWriter {
    file: File,
    cache: Arc<TrackCache>,
    track_id: String,
    partial: PathBuf,
    final_path: PathBuf,
    content_length: Option<u64>,
    promoted: bool,
}

impl CacheWriter {
    /// Renames `.part` to its final name once the file is complete.
    ///
    /// Length is checked against the server's content length rather than
    /// counting writes, because the writer can be seeked: a byte count would
    /// over-report on any retry or out-of-order fill.
    fn promote_if_complete(&mut self) {
        if self.promoted {
            return;
        }
        let Some(expected) = self.content_length else {
            // Without a content length there is no way to know we have it all,
            // so the track streams but is never cached.
            return;
        };
        let Ok(meta) = self.file.metadata() else {
            return;
        };
        if meta.len() < expected {
            return;
        }

        // Renaming a file that is still open is fine on Unix: existing handles
        // keep pointing at the same inode.
        if let Err(err) = std::fs::rename(&self.partial, &self.final_path) {
            log::warn!("could not promote {}: {err}", self.partial.display());
            return;
        }
        self.promoted = true;
        self.cache.insert(&self.track_id, &self.final_path);
        log::debug!("cached {} ({} bytes)", self.track_id, expected);
    }
}

impl Write for CacheWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.promote_if_complete();
        Ok(())
    }
}

impl Seek for CacheWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Read for CacheWriter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("navidrome-rs-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn write_track(cache: &TrackCache, id: &str, bytes: usize) -> PathBuf {
        let path = cache.path_for(id, Some("flac"));
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        cache.insert(id, &path);
        path
    }

    #[test]
    fn file_names_are_path_safe() {
        // Subsonic ids are opaque; a server could return something with a slash.
        let name = TrackCache::file_name("../../etc/passwd", Some("flac"));
        assert!(!name.contains('/'), "unsafe cache file name: {name}");
        assert!(name.ends_with(".flac"));
    }

    #[test]
    fn lookup_misses_when_absent_and_hits_once_written() {
        let dir = temp_dir("lookup");
        let cache = TrackCache::new(&dir, 1024).unwrap();
        assert!(cache.lookup("a", Some("flac")).is_none());

        write_track(&cache, "a", 10);
        assert!(cache.lookup("a", Some("flac")).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let dir = temp_dir("evict");
        let cache = TrackCache::new(&dir, 250).unwrap();

        write_track(&cache, "old", 100);
        write_track(&cache, "mid", 100);
        // Touch "old" so "mid" becomes the least recently used.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        cache.lookup("old", Some("flac"));

        // Pushes the total to 300, over the 250 budget.
        write_track(&cache, "new", 100);

        assert!(cache.total_bytes() <= 250, "cache exceeded its budget");
        assert!(cache.lookup("new", Some("flac")).is_some(), "newest evicted");
        assert!(
            cache.lookup("mid", Some("flac")).is_none(),
            "least recently used should have gone first"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_partial_files_are_cleaned_up_on_open() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("something.flac.part");
        std::fs::write(&stale, b"half a song").unwrap();

        let _cache = TrackCache::new(&dir, 1024).unwrap();

        assert!(!stale.exists(), "interrupted download was left behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn index_survives_reopening() {
        let dir = temp_dir("reopen");
        {
            let cache = TrackCache::new(&dir, 1024).unwrap();
            write_track(&cache, "keep", 42);
        }
        let cache = TrackCache::new(&dir, 1024).unwrap();
        assert_eq!(cache.total_bytes(), 42);
        assert!(cache.lookup("keep", Some("flac")).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
