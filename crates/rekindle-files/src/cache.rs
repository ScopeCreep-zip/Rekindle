//! Filesystem-backed chunk cache with LRU + pinned-skip eviction.
//!
//! Layout (plan E3.0a):
//! ```text
//! state/{pk}.files/<scope name>/<attachment name>/<chunk_index>.bin
//! ```
//! Both names are keyed hashes ([`names`]): `<scope name>` of the scope
//! ([`scope_root`], the caller's `root_dir`) and `<attachment name>` of the
//! attachment id bound to that scope. Before unlock the tree shows only
//! sizes and counts. Each attachment gets its own directory holding all of
//! its cached chunks. Eviction removes individual `*.bin` files; an
//! attachment directory left empty is removed on the next `open`.

pub mod names;
pub mod store;

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use lru::LruCache;
use rekindle_types::attachment::AttachmentBitmap;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::FilesError;
use crate::pinned::PinnedSet;

pub use names::{keyed_name, scope_root, AttachmentName, CacheScope};
pub use store::FileCacheStore;

#[derive(Clone)]
pub struct CacheConfig {
    /// One scope's directory, `scope_root(identity_files, key, scope)`.
    pub root_dir: PathBuf,
    /// Soft byte budget. Eviction runs after every successful `insert` to
    /// bring `total_bytes` under this number.
    pub byte_budget: u64,
    /// The identity's file-cache name key
    /// (`rekindle_secrets::derive::derive_file_cache_name_key`): attachment
    /// directories are named with it.
    pub name_key: Zeroizing<[u8; 32]>,
}

impl CacheConfig {
    pub const DEFAULT_BYTE_BUDGET: u64 = 1024 * 1024 * 1024; // 1 GB per spec §28.9
}

impl std::fmt::Debug for CacheConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheConfig")
            .field("root_dir", &self.root_dir)
            .field("byte_budget", &self.byte_budget)
            .field("name_key", &"<redacted>")
            .finish()
    }
}

/// LRU + pinned-skip chunk cache backed by the filesystem.
///
/// The LRU is in-memory — on `open()` we walk the cache directory and
/// register every existing chunk as a "least-recently-used" entry (since
/// we have no on-disk access timestamps to honour). Subsequent `get`/`insert`
/// updates the LRU normally. Crashes or kills lose only the LRU ordering;
/// chunks themselves are durable. The LRU is keyed by the attachment's
/// directory name, since the id cannot be read back from the disk; every
/// lookup names the attachment first.
pub struct ChunkCache {
    config: CacheConfig,
    /// The scope directory's own name, bound into every attachment name.
    scope_dir: Vec<u8>,
    /// (attachment name, chunk_index) → byte size on disk.
    /// Wrapped in NonZero capacity since LruCache requires it; we set a
    /// generous cap (1M entries) — the byte budget is the real limit.
    lru: LruCache<(AttachmentName, u32), u64>,
    total_bytes: u64,
}

impl ChunkCache {
    pub fn open(config: CacheConfig) -> Result<Self, FilesError> {
        std::fs::create_dir_all(&config.root_dir)
            .map_err(|e| FilesError::io(config.root_dir.display().to_string(), e))?;

        // Walk the cache to populate LRU + total_bytes.
        let mut lru: LruCache<(AttachmentName, u32), u64> =
            LruCache::new(NonZeroUsize::new(1_000_000).expect("non-zero"));
        let mut total_bytes: u64 = 0;
        let mut existing: Vec<(AttachmentName, u32, u64)> = Vec::new();

        // One level of attachment directories: <root>/<name>/<idx>.bin
        let outer = std::fs::read_dir(&config.root_dir)
            .map_err(|e| FilesError::io(config.root_dir.display().to_string(), e))?;
        for attach_entry in outer {
            let attach_entry = attach_entry
                .map_err(|e| FilesError::io(config.root_dir.display().to_string(), e))?;
            if !attach_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let attach_path = attach_entry.path();
            let Some(name) = attach_path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(AttachmentName::parse)
            else {
                continue;
            };
            let found = walk_chunks(&attach_path, name, &mut existing)?;
            if found.0 == 0 {
                // Every chunk was evicted: drop the directory (a stray file
                // keeps it, `remove_dir` refuses a non-empty one).
                let _ = std::fs::remove_dir(&attach_path);
            }
            total_bytes = total_bytes.saturating_add(found.1);
        }

        // Seed LRU oldest-first so the order matches "least recently used"
        // (we have no real access timestamps, so this is arbitrary but
        // deterministic by directory iteration order).
        for (name, idx, size) in existing {
            lru.put((name, idx), size);
        }

        let scope_dir = config
            .root_dir
            .file_name()
            .map(|n| n.as_encoded_bytes().to_vec())
            .unwrap_or_default();
        Ok(Self {
            config,
            scope_dir,
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

    /// The directory name of `attachment_id` in this scope.
    #[must_use]
    pub fn attachment_name(&self, attachment_id: Uuid) -> AttachmentName {
        AttachmentName::derive(
            &self.config.name_key,
            &self.scope_dir,
            attachment_id.as_bytes(),
        )
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
        let name = self.attachment_name(attachment_id);
        let path = self.chunk_path(name, chunk_index);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| FilesError::io(parent.display().to_string(), e))?;
        }
        std::fs::write(&path, ciphertext)
            .map_err(|e| FilesError::io(path.display().to_string(), e))?;

        let new_len = ciphertext.len() as u64;
        if let Some(old_len) = self.lru.put((name, chunk_index), new_len) {
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
        let key = (self.attachment_name(attachment_id), chunk_index);
        if self.lru.get(&key).is_none() {
            return Ok(None);
        }
        let path = self.chunk_path(key.0, chunk_index);
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
        let name = self.attachment_name(attachment_id);
        self.remove_named(name, chunk_index)
    }

    fn remove_named(&mut self, name: AttachmentName, chunk_index: u32) -> Result<(), FilesError> {
        if let Some(old_len) = self.lru.pop(&(name, chunk_index)) {
            self.total_bytes = self.total_bytes.saturating_sub(old_len);
        }
        let path = self.chunk_path(name, chunk_index);
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
        let name = self.attachment_name(attachment_id);
        let mut bm = AttachmentBitmap::new(chunk_count);
        for (key, _) in &self.lru {
            if key.0 == name && key.1 < chunk_count && !bm.set(key.1) {
                return Err(FilesError::ChunkIndexOutOfRange {
                    index: key.1,
                    chunk_count,
                });
            }
        }
        Ok(bm)
    }

    /// Whether at least one chunk of `attachment_id` is cached here.
    #[must_use]
    pub fn holds(&self, attachment_id: Uuid) -> bool {
        let name = self.attachment_name(attachment_id);
        self.lru.iter().any(|(key, _)| key.0 == name)
    }

    /// Walk LRU oldest-first and remove non-pinned chunks until
    /// `total_bytes ≤ byte_budget`. Returns the number of bytes evicted.
    /// Pinned attachments are skipped — if every entry is pinned and we
    /// still exceed the budget, eviction is a no-op (admin overcommit).
    pub fn evict_to_budget(&mut self, pinned: &PinnedSet) -> Result<u64, FilesError> {
        if self.total_bytes <= self.config.byte_budget {
            return Ok(0);
        }

        let pinned: HashSet<AttachmentName> =
            pinned.iter().map(|id| self.attachment_name(*id)).collect();
        let mut evicted: u64 = 0;
        // Build a snapshot of LRU order (oldest first) so we don't borrow
        // self.lru immutably while we mutate it.
        let order: Vec<(AttachmentName, u32)> = self
            .lru
            .iter()
            .rev() // LruCache::iter is most-recent-first; we want oldest-first.
            .map(|((name, idx), _)| (*name, *idx))
            .collect();

        for (name, idx) in order {
            if self.total_bytes <= self.config.byte_budget {
                break;
            }
            if pinned.contains(&name) {
                continue;
            }
            // remove_named() updates LRU + total_bytes + filesystem
            let before = self.total_bytes;
            self.remove_named(name, idx)?;
            evicted = evicted.saturating_add(before.saturating_sub(self.total_bytes));
        }

        Ok(evicted)
    }

    fn chunk_path(&self, name: AttachmentName, chunk_index: u32) -> PathBuf {
        self.config
            .root_dir
            .join(name.to_hex())
            .join(format!("{chunk_index}.bin"))
    }
}

/// Register every `<idx>.bin` in one attachment directory; returns how many
/// chunks it held and their bytes.
fn walk_chunks(
    attach_path: &Path,
    name: AttachmentName,
    existing: &mut Vec<(AttachmentName, u32, u64)>,
) -> Result<(u32, u64), FilesError> {
    let Ok(chunks) = std::fs::read_dir(attach_path) else {
        return Ok((0, 0));
    };
    let (mut count, mut bytes) = (0u32, 0u64);
    for chunk_entry in chunks {
        let chunk_entry =
            chunk_entry.map_err(|e| FilesError::io(attach_path.display().to_string(), e))?;
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
        existing.push((name, chunk_index, len));
        count += 1;
        bytes = bytes.saturating_add(len);
    }
    Ok((count, bytes))
}

#[cfg(test)]
mod tests;
