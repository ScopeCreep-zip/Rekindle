//! Filesystem-backed chunk cache with LRU + pinned-skip eviction.
//!
//! Layout (plan §1.J3):
//! ```text
//! <root_dir>/<aa>/<full_attachment_hex>/<chunk_index>.bin
//! ```
//! `<aa>` = first 2 hex characters of the attachment id (256-way fanout,
//! git-style). Each attachment gets its own directory holding all of its
//! cached chunks. Eviction removes individual `*.bin` files; an attachment
//! directory becomes empty naturally when all chunks are evicted (we
//! garbage-collect empty dirs lazily on next `open`).

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use lru::LruCache;
use rekindle_types::attachment::AttachmentBitmap;
use uuid::Uuid;

use crate::error::FilesError;
use crate::pinned::PinnedSet;

#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Root directory — typically `<app_data>/file_cache/<community_id>`.
    /// The caller is responsible for namespacing per community.
    pub root_dir: PathBuf,
    /// Soft byte budget. Eviction runs after every successful `insert` to
    /// bring `total_bytes` under this number.
    pub byte_budget: u64,
}

impl CacheConfig {
    pub const DEFAULT_BYTE_BUDGET: u64 = 1024 * 1024 * 1024; // 1 GB per spec §28.9
}

/// LRU + pinned-skip chunk cache backed by the filesystem.
///
/// The LRU is in-memory — on `open()` we walk the cache directory and
/// register every existing chunk as a "least-recently-used" entry (since
/// we have no on-disk access timestamps to honour). Subsequent `get`/`insert`
/// updates the LRU normally. Crashes or kills lose only the LRU ordering;
/// chunks themselves are durable.
pub struct ChunkCache {
    config: CacheConfig,
    /// (attachment_id, chunk_index) → byte size on disk.
    /// Wrapped in NonZero capacity since LruCache requires it; we set a
    /// generous cap (1M entries) — the byte budget is the real limit.
    lru: LruCache<(Uuid, u32), u64>,
    total_bytes: u64,
}

impl ChunkCache {
    pub fn open(config: CacheConfig) -> Result<Self, FilesError> {
        std::fs::create_dir_all(&config.root_dir)
            .map_err(|e| FilesError::io(config.root_dir.display().to_string(), e))?;

        // Walk the cache to populate LRU + total_bytes.
        let mut lru: LruCache<(Uuid, u32), u64> =
            LruCache::new(NonZeroUsize::new(1_000_000).expect("non-zero"));
        let mut total_bytes: u64 = 0;
        let mut existing: Vec<(Uuid, u32, u64)> = Vec::new();

        // Two-level walk: <root>/<aa>/<full_hex>/<idx>.bin
        let outer = match std::fs::read_dir(&config.root_dir) {
            Ok(it) => it,
            Err(e) => return Err(FilesError::io(config.root_dir.display().to_string(), e)),
        };
        for fanout_entry in outer {
            let fanout_entry = fanout_entry
                .map_err(|e| FilesError::io(config.root_dir.display().to_string(), e))?;
            if !fanout_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let fanout_path = fanout_entry.path();
            let inner = match std::fs::read_dir(&fanout_path) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for attach_entry in inner {
                let attach_entry = attach_entry
                    .map_err(|e| FilesError::io(fanout_path.display().to_string(), e))?;
                let attach_path = attach_entry.path();
                let Some(hex_name) = attach_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Ok(attachment_id) = parse_attachment_id_hex(hex_name) else {
                    continue;
                };
                let chunks = match std::fs::read_dir(&attach_path) {
                    Ok(it) => it,
                    Err(_) => continue,
                };
                for chunk_entry in chunks {
                    let chunk_entry = chunk_entry
                        .map_err(|e| FilesError::io(attach_path.display().to_string(), e))?;
                    let chunk_path = chunk_entry.path();
                    let Some(stem) = chunk_path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .filter(|_| chunk_path.extension().and_then(|e| e.to_str()) == Some("bin"))
                    else {
                        continue;
                    };
                    let Ok(chunk_index) = stem.parse::<u32>() else {
                        continue;
                    };
                    let len = chunk_entry.metadata().map(|m| m.len()).unwrap_or(0);
                    existing.push((attachment_id, chunk_index, len));
                    total_bytes = total_bytes.saturating_add(len);
                }
            }
        }

        // Seed LRU oldest-first so the order matches "least recently used"
        // (we have no real access timestamps, so this is arbitrary but
        // deterministic by directory iteration order).
        for (id, idx, size) in existing {
            lru.put((id, idx), size);
        }

        Ok(Self {
            config,
            lru,
            total_bytes,
        })
    }

    /// Configured byte budget.
    pub fn byte_budget(&self) -> u64 {
        self.config.byte_budget
    }

    /// Current bytes-on-disk in the cache (sum of all chunk file sizes).
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Number of chunks currently cached across all attachments.
    pub fn chunk_count(&self) -> usize {
        self.lru.len()
    }

    /// Insert (or overwrite) a chunk's ciphertext bytes. Updates LRU,
    /// then runs eviction until `total_bytes ≤ byte_budget`, skipping
    /// pinned attachments.
    pub fn insert(
        &mut self,
        attachment_id: Uuid,
        chunk_index: u32,
        ciphertext: &[u8],
        pinned: &PinnedSet,
    ) -> Result<(), FilesError> {
        let path = self.chunk_path(attachment_id, chunk_index);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| FilesError::io(parent.display().to_string(), e))?;
        }
        std::fs::write(&path, ciphertext)
            .map_err(|e| FilesError::io(path.display().to_string(), e))?;

        let new_len = ciphertext.len() as u64;
        let key = (attachment_id, chunk_index);
        if let Some(old_len) = self.lru.put(key, new_len) {
            self.total_bytes = self.total_bytes.saturating_sub(old_len);
        }
        self.total_bytes = self.total_bytes.saturating_add(new_len);

        self.evict_to_budget(pinned)?;
        Ok(())
    }

    /// Read a chunk's ciphertext if present. Updates LRU recency.
    pub fn get(
        &mut self,
        attachment_id: Uuid,
        chunk_index: u32,
    ) -> Result<Option<Vec<u8>>, FilesError> {
        let key = (attachment_id, chunk_index);
        if self.lru.get(&key).is_none() {
            return Ok(None);
        }
        let path = self.chunk_path(attachment_id, chunk_index);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // LRU and disk diverged — drop the stale entry and report missing.
                if let Some(stale) = self.lru.pop(&key) {
                    self.total_bytes = self.total_bytes.saturating_sub(stale);
                }
                Ok(None)
            }
            Err(e) => Err(FilesError::io(path.display().to_string(), e)),
        }
    }

    /// Forcibly remove a single chunk file. Idempotent — missing chunks
    /// are not an error.
    pub fn remove(&mut self, attachment_id: Uuid, chunk_index: u32) -> Result<(), FilesError> {
        let key = (attachment_id, chunk_index);
        if let Some(old_len) = self.lru.pop(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old_len);
        }
        let path = self.chunk_path(attachment_id, chunk_index);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FilesError::io(path.display().to_string(), e)),
        }
    }

    /// Build the current bitmap for an attachment by scanning the LRU
    /// (no disk I/O — relies on the open-time scan being current).
    pub fn bitmap_for(
        &self,
        attachment_id: Uuid,
        chunk_count: u32,
    ) -> Result<AttachmentBitmap, FilesError> {
        let mut bm = AttachmentBitmap::new(chunk_count);
        for (key, _) in self.lru.iter() {
            if key.0 == attachment_id && key.1 < chunk_count && !bm.set(key.1) {
                return Err(FilesError::ChunkIndexOutOfRange {
                    index: key.1,
                    chunk_count,
                });
            }
        }
        Ok(bm)
    }

    /// Walk LRU oldest-first and remove non-pinned chunks until
    /// `total_bytes ≤ byte_budget`. Returns the number of bytes evicted.
    /// Pinned attachments are skipped — if every entry is pinned and we
    /// still exceed the budget, eviction is a no-op (admin overcommit).
    pub fn evict_to_budget(&mut self, pinned: &PinnedSet) -> Result<u64, FilesError> {
        if self.total_bytes <= self.config.byte_budget {
            return Ok(0);
        }

        let mut evicted: u64 = 0;
        // Build a snapshot of LRU order (oldest first) so we don't borrow
        // self.lru immutably while we mutate it.
        let order: Vec<(Uuid, u32, u64)> = self
            .lru
            .iter()
            .rev() // LruCache::iter is most-recent-first; we want oldest-first.
            .map(|((id, idx), size)| (*id, *idx, *size))
            .collect();

        for (id, idx, _size) in order {
            if self.total_bytes <= self.config.byte_budget {
                break;
            }
            if pinned.contains(&id) {
                continue;
            }
            // remove() updates LRU + total_bytes + filesystem
            let before = self.total_bytes;
            self.remove(id, idx)?;
            evicted = evicted.saturating_add(before.saturating_sub(self.total_bytes));
        }

        Ok(evicted)
    }

    /// Statistics aggregated per attachment (chunk count + total bytes).
    pub fn stats_per_attachment(&self) -> HashMap<Uuid, (u32, u64)> {
        let mut out: HashMap<Uuid, (u32, u64)> = HashMap::new();
        for ((id, _idx), size) in self.lru.iter() {
            let entry = out.entry(*id).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(*size);
        }
        out
    }

    fn chunk_path(&self, attachment_id: Uuid, chunk_index: u32) -> PathBuf {
        let hex = uuid_to_hex(attachment_id);
        let fanout: String = hex.chars().take(2).collect();
        self.config
            .root_dir
            .join(fanout)
            .join(hex)
            .join(format!("{chunk_index}.bin"))
    }
}

fn uuid_to_hex(id: Uuid) -> String {
    hex::encode(id.as_bytes())
}

fn parse_attachment_id_hex(hex_str: &str) -> Result<Uuid, FilesError> {
    let bytes =
        hex::decode(hex_str).map_err(|_| FilesError::InvalidAttachmentId(hex_str.to_string()))?;
    let arr: [u8; 16] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| FilesError::InvalidAttachmentId(hex_str.to_string()))?;
    Ok(Uuid::from_bytes(arr))
}

/// Build a per-community sub-cache root path: `<base>/<community_id>`.
/// Helper exposed so the Tauri shell can compute the same paths.
pub fn community_cache_root(base: &Path, community_id: &str) -> PathBuf {
    base.join(community_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cache(temp: &TempDir, budget: u64) -> ChunkCache {
        ChunkCache::open(CacheConfig {
            root_dir: temp.path().to_path_buf(),
            byte_budget: budget,
        })
        .unwrap()
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 1024 * 1024);
        let pinned = PinnedSet::new();
        let id = Uuid::new_v4();
        c.insert(id, 0, b"hello", &pinned).unwrap();
        assert_eq!(c.get(id, 0).unwrap().as_deref(), Some(&b"hello"[..]));
        assert_eq!(c.total_bytes(), 5);
    }

    #[test]
    fn missing_chunk_returns_none() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 1024);
        let id = Uuid::new_v4();
        assert!(c.get(id, 0).unwrap().is_none());
    }

    #[test]
    fn lru_evicts_when_over_budget() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 30); // tight budget
        let pinned = PinnedSet::new();
        let id = Uuid::new_v4();
        c.insert(id, 0, &[0u8; 10], &pinned).unwrap();
        c.insert(id, 1, &[0u8; 10], &pinned).unwrap();
        c.insert(id, 2, &[0u8; 10], &pinned).unwrap();
        // total = 30, exactly at budget; no eviction yet.
        assert_eq!(c.total_bytes(), 30);
        c.insert(id, 3, &[0u8; 10], &pinned).unwrap();
        // Inserted +10 → 40 → exceeded → evicted oldest until ≤ 30.
        assert!(c.total_bytes() <= 30);
        // Most recent should still be present.
        assert!(c.get(id, 3).unwrap().is_some());
    }

    #[test]
    fn pinned_chunks_survive_eviction() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 25);
        let mut pinned = PinnedSet::new();
        let pinned_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();
        pinned.insert(pinned_id);

        c.insert(pinned_id, 0, &[0u8; 10], &pinned).unwrap();
        c.insert(pinned_id, 1, &[0u8; 10], &pinned).unwrap();
        c.insert(other_id, 0, &[0u8; 10], &pinned).unwrap();
        // 30 bytes > 25 → evict — must skip the pinned ones.
        assert!(c.get(pinned_id, 0).unwrap().is_some());
        assert!(c.get(pinned_id, 1).unwrap().is_some());
        assert!(c.get(other_id, 0).unwrap().is_none());
    }

    #[test]
    fn bitmap_reflects_inserts() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 1024);
        let pinned = PinnedSet::new();
        let id = Uuid::new_v4();
        c.insert(id, 0, b"a", &pinned).unwrap();
        c.insert(id, 3, b"b", &pinned).unwrap();
        c.insert(id, 7, b"c", &pinned).unwrap();
        let bm = c.bitmap_for(id, 8).unwrap();
        assert!(bm.has(0));
        assert!(bm.has(3));
        assert!(bm.has(7));
        assert!(!bm.has(1));
        assert_eq!(bm.count(), 3);
    }

    #[test]
    fn open_rebuilds_from_disk() {
        let temp = TempDir::new().unwrap();
        let pinned = PinnedSet::new();
        let id = Uuid::new_v4();
        {
            let mut c = cache(&temp, 1024);
            c.insert(id, 0, b"persisted", &pinned).unwrap();
            c.insert(id, 5, b"more", &pinned).unwrap();
        }
        // Re-open and confirm chunks are still discoverable.
        let mut c2 = cache(&temp, 1024);
        assert_eq!(c2.total_bytes(), 9 + 4);
        assert_eq!(c2.get(id, 0).unwrap().as_deref(), Some(&b"persisted"[..]));
        assert_eq!(c2.get(id, 5).unwrap().as_deref(), Some(&b"more"[..]));
    }

    #[test]
    fn remove_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let mut c = cache(&temp, 1024);
        let pinned = PinnedSet::new();
        let id = Uuid::new_v4();
        c.insert(id, 0, b"x", &pinned).unwrap();
        c.remove(id, 0).unwrap();
        c.remove(id, 0).unwrap(); // second remove is fine
        assert_eq!(c.total_bytes(), 0);
        assert!(c.get(id, 0).unwrap().is_none());
    }
}
