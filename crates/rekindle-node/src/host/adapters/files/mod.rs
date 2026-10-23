//! The Host's Lost Cargo chunk caches (plan E3.1 E3c), on both modes: the
//! one [`FileCacheStore`] (`HostStores::files`, the GUI's `AppState.files`
//! in Shell mode) is opened at unlock over the identity's keyed files
//! directory (plan E3.0a: `DataRoot::identity_files` and the file-cache
//! name key) and closed when the unlock scope has stopped. [`HostFiles`]
//! is the `rekindle_files::FileCacheDeps` the expression store runs over,
//! and the full `FilesDeps` the attachment upload, download, serve and the
//! eager expression fetch run over (plan E3.1 E3d; `deps_impl`, `rows`,
//! `ops`).

mod deps_impl;
pub mod ops;
mod rows;

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
    channel: crate::host::adapters::channel::HostChannel,
    store: Arc<FileCacheStore>,
}

impl crate::host::Host {
    /// The attachment and chunk-cache adapter over this Host.
    #[must_use]
    pub fn files(&self) -> HostFiles {
        HostFiles::new(crate::daemon::dispatch::adapter(&self.ctx))
    }
}

impl HostFiles {
    pub(crate) fn new(gov: HostGovernance) -> Self {
        let store = Arc::clone(&gov.context().files);
        let channel = crate::host::adapters::channel::HostChannel::new(gov.context());
        Self {
            gov,
            channel,
            store,
        }
    }

    /// The record pool of the running transport.
    fn records(&self) -> Result<Arc<dyn rekindle_records::port::RecordPort>, FilesError> {
        let transport = (self.gov.context().transport.read().clone())
            .ok_or_else(|| FilesError::Transport("transport not started".into()))?;
        (transport.require_records())
            .map_err(|e| FilesError::Transport(format!("record pool: {e}")))
    }

    /// The unlocked identity's database.
    fn db(&self) -> Result<rekindle_db::Db, FilesError> {
        (self.gov.context().db.current()).map_err(|e| FilesError::Db(e.to_string()))
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
