//! The Host's Lost Cargo chunk caches (plan E3.1 E3c), on both modes: the
//! one [`FileCacheStore`] (`HostStores::files`, the GUI's `AppState.files`
//! in Shell mode) is opened at unlock over the identity's keyed files
//! directory (plan E3.0a: `DataRoot::identity_files` and the file-cache
//! name key) and closed when the unlock scope has stopped. [`HostFiles`]
//! is the `rekindle_files::FileCacheDeps` the expression store runs over.

use std::sync::Arc;

use rekindle_files::{ChunkCache, FileCacheDeps, FileCacheStore, FilesError, PinnedSet};
use rekindle_lifecycle::SessionScope;
use uuid::Uuid;

use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::governance::HostGovernance;

/// Open the store for the unlocked identity, under the unlock `scope`: a
/// scope already stopping opens nothing.
///
/// # Errors
/// The unlock already ended, or no identity is unlocked.
pub(crate) fn open(ctx: &DaemonContext, scope: &Arc<SessionScope>) -> Result<(), String> {
    if scope.token().is_cancelled() {
        return Err("the unlock already ended".into());
    }
    let owner = ctx.require_owner().map_err(|_| "no identity is unlocked")?;
    let secret = zeroize::Zeroizing::new(
        ctx.require_signing_key()
            .map_err(|_| "no identity is unlocked")?,
    );
    let name_key = rekindle_secrets::derive::derive_file_cache_name_key(&secret);
    ctx.files.open(ctx.root.identity_files(&owner), name_key);
    Ok(())
}

/// Close the store (the unlock's tasks have stopped).
pub(crate) fn close(ctx: &DaemonContext) {
    ctx.files.close();
}

/// The Host's chunk caches and community keys, as the expression store
/// sees them.
#[derive(Clone)]
pub struct HostFiles {
    gov: HostGovernance,
    store: Arc<FileCacheStore>,
}

impl HostFiles {
    pub(crate) fn new(gov: HostGovernance) -> Self {
        let store = Arc::clone(&gov.context().files);
        Self { gov, store }
    }
}

impl FileCacheDeps for HostFiles {
    fn keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        rekindle_governance_runtime::deps::GovernanceRuntimeDeps::keys(&self.gov)
    }

    fn ensure_cache_open(&self, community_id: &str) -> Result<(), FilesError> {
        self.store.ensure_open(community_id)
    }

    fn pinned_attachments(&self, community_id: &str) -> PinnedSet {
        self.store.pinned(community_id)
    }

    fn set_community_pinned_set(&self, community_id: &str, ids: Vec<Uuid>) {
        self.store.set_pinned(community_id, ids);
    }

    fn with_cache_mut(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&mut ChunkCache, &PinnedSet) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        self.store.with_cache_mut(community_id, f)
    }

    fn with_cache(
        &self,
        community_id: &str,
        f: &mut dyn FnMut(&ChunkCache) -> Result<(), FilesError>,
    ) -> Result<(), FilesError> {
        self.store.with_cache(community_id, f)
    }
}
