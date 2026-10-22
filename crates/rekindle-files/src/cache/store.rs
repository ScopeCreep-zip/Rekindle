//! The unlocked identity's chunk caches (plan E3.1 E3c): one store per
//! host, opened at unlock with the identity's keyed files directory and
//! name key (plan E3.0a) and closed at its end, so no cache outlives the
//! session that keyed it. Each community's [`ChunkCache`] opens on first
//! use under its keyed scope directory.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::RwLock;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{scope_root, CacheConfig, CacheScope, ChunkCache};
use crate::error::FilesError;
use crate::pinned::PinnedSet;

/// One unlock's caches.
struct Open {
    identity_files: PathBuf,
    name_key: Zeroizing<[u8; 32]>,
    caches: HashMap<String, ChunkCache>,
    pinned: HashMap<String, PinnedSet>,
}

/// The host's chunk caches, empty while locked.
#[derive(Default)]
pub struct FileCacheStore {
    open: RwLock<Option<Open>>,
}

impl std::fmt::Debug for FileCacheStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileCacheStore")
            .field("open", &self.open.read().is_some())
            .finish()
    }
}

fn locked() -> FilesError {
    FilesError::IdentityNotLoaded
}

impl FileCacheStore {
    /// Open the store for an unlocked identity: `identity_files` is its
    /// `DataRoot::identity_files`, `name_key` its file-cache name key.
    pub fn open(&self, identity_files: PathBuf, name_key: Zeroizing<[u8; 32]>) {
        *self.open.write() = Some(Open {
            identity_files,
            name_key,
            caches: HashMap::new(),
            pinned: HashMap::new(),
        });
    }

    /// Close every cache and forget the key (the session ended).
    pub fn close(&self) {
        *self.open.write() = None;
    }

    /// Open `community_id`'s cache if it is not open yet.
    ///
    /// # Errors
    /// [`FilesError::IdentityNotLoaded`] while locked, or the cache's
    /// directory could not be opened.
    pub fn ensure_open(&self, community_id: &str) -> Result<(), FilesError> {
        let mut guard = self.open.write();
        let open = guard.as_mut().ok_or_else(locked)?;
        if open.caches.contains_key(community_id) {
            return Ok(());
        }
        let config = CacheConfig {
            root_dir: scope_root(
                &open.identity_files,
                &open.name_key,
                CacheScope::Community(community_id),
            ),
            byte_budget: CacheConfig::DEFAULT_BYTE_BUDGET,
            name_key: open.name_key.clone(),
        };
        let cache = ChunkCache::open(config)?;
        open.caches.insert(community_id.to_string(), cache);
        open.pinned.entry(community_id.to_string()).or_default();
        Ok(())
    }

    /// Run `f` over `community_id`'s open cache and its pinned set.
    ///
    /// # Errors
    /// The cache is not open, or `f`'s error.
    pub fn with_cache_mut(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&mut ChunkCache, &PinnedSet) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let mut guard = self.open.write();
        let open = guard.as_mut().ok_or_else(locked)?;
        let pinned = open.pinned.get(community_id).cloned().unwrap_or_default();
        let cache = open
            .caches
            .get_mut(community_id)
            .ok_or_else(|| FilesError::NotFound(format!("file cache for {community_id}")))?;
        f(cache, &pinned)
    }

    /// Run `f` over `community_id`'s open cache.
    ///
    /// # Errors
    /// The cache is not open, or `f`'s error.
    pub fn with_cache(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&ChunkCache) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        let guard = self.open.read();
        let open = guard.as_ref().ok_or_else(locked)?;
        let cache = open
            .caches
            .get(community_id)
            .ok_or_else(|| FilesError::NotFound(format!("file cache for {community_id}")))?;
        f(cache)
    }

    /// `community_id`'s pinned attachments (empty while locked).
    #[must_use]
    pub fn pinned(&self, community_id: &str) -> PinnedSet {
        (self.open.read().as_ref())
            .and_then(|open| open.pinned.get(community_id).cloned())
            .unwrap_or_default()
    }

    /// Replace `community_id`'s pinned attachments (no-op while locked).
    pub fn set_pinned(&self, community_id: &str, ids: Vec<Uuid>) {
        if let Some(open) = self.open.write().as_mut() {
            open.pinned
                .entry(community_id.to_string())
                .or_default()
                .replace(ids);
        }
    }

    /// The community whose open cache holds `attachment_id`.
    #[must_use]
    pub fn holder_of(&self, attachment_id: Uuid) -> Option<String> {
        let guard = self.open.read();
        let open = guard.as_ref()?;
        open.caches
            .iter()
            .find_map(|(cid, cache)| cache.holds(attachment_id).then(|| cid.clone()))
    }

    /// Answer a peer's chunk request from `community_id`'s cache.
    #[must_use]
    pub fn serve(
        &self,
        community_id: &str,
        attachment_id: [u8; 16],
        requested_chunks: &[u32],
    ) -> Option<Vec<u8>> {
        let mut guard = self.open.write();
        let cache = guard.as_mut()?.caches.get_mut(community_id)?;
        crate::serve::serve_attachment_request(cache, attachment_id, requested_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_store_refuses_and_close_forgets() {
        let store = FileCacheStore::default();
        assert!(matches!(
            store.ensure_open("c"),
            Err(FilesError::IdentityNotLoaded)
        ));
        let dir = tempfile::tempdir().expect("tempdir");
        store.open(dir.path().to_path_buf(), Zeroizing::new([7u8; 32]));
        store.ensure_open("c").expect("opens");
        let id = Uuid::from_bytes([1; 16]);
        store
            .with_cache_mut("c", &mut |cache, pinned| {
                cache.insert(id, 0, b"chunk", pinned)
            })
            .expect("insert");
        assert_eq!(store.holder_of(id).as_deref(), Some("c"));
        store.close();
        assert_eq!(store.holder_of(id), None);
        assert!(store.with_cache("c", &mut |_| Ok(())).is_err());
    }
}
