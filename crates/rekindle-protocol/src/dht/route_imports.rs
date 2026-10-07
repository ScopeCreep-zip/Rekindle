//! `RouteImports`: the only importer of peers' private routes (plan C7.2, D4).
//! One per node, for the life of the process.
//!
//! veilid-core's import is idempotent: "It is safe to import the same route
//! more than once and it will return the same route id"
//! (`route_spec_store/route_remote.rs:21-28`). Imported routes live in a
//! 1024-entry LRU with a 5-minute idle expiry that Veilid owns
//! (`route_spec_store/mod.rs:39-42`). Releasing an imported id removes the
//! **shared** entry for every sender in the process and reports it as a dead
//! remote route (`route_spec_store_cache.rs:398-415`). So this cache has no
//! TTL and never releases: a dead route (reported, or failed on send) is only
//! forgotten, since Veilid releases a dead remote route itself
//! (`ping_validator.rs:610-628`) and a second release is an ERROR (plan
//! C7.9e, amending C7.2's single release).

use std::collections::HashMap;

use parking_lot::Mutex;
use veilid_core::{RouteId, VeilidAPI};

use crate::ProtocolError;

/// Imported route ids, by the blob they came from.
#[derive(Debug)]
pub struct RouteImports {
    api: VeilidAPI,
    by_blob: Mutex<HashMap<Vec<u8>, RouteId>>,
    by_id: Mutex<HashMap<RouteId, Vec<u8>>>,
}

impl RouteImports {
    #[must_use]
    pub fn new(api: VeilidAPI) -> Self {
        Self {
            api,
            by_blob: Mutex::new(HashMap::new()),
            by_id: Mutex::new(HashMap::new()),
        }
    }

    /// The route id for a peer's route blob, importing it the first time.
    ///
    /// # Errors
    /// The blob is not a valid route.
    pub fn get_or_import(&self, blob: &[u8]) -> Result<RouteId, ProtocolError> {
        if let Some(id) = self.by_blob.lock().get(blob) {
            return Ok(id.clone());
        }
        let id = self
            .api
            .import_remote_private_route(blob.to_vec())
            .map_err(|e| ProtocolError::RoutingError(format!("import route: {e}")))?;
        self.by_blob.lock().insert(blob.to_vec(), id.clone());
        self.by_id.lock().insert(id.clone(), blob.to_vec());
        Ok(id)
    }

    /// Veilid reported these imported routes dead (`RouteChange`): forget
    /// them, without releasing (Veilid already dropped them). Returns their
    /// blobs so the caller can re-fetch the peers' current routes.
    pub fn on_dead_remote(&self, dead: &[RouteId]) -> Vec<Vec<u8>> {
        let mut by_id = self.by_id.lock();
        let mut by_blob = self.by_blob.lock();
        dead.iter()
            .filter_map(|id| by_id.remove(id))
            .inspect(|blob| {
                by_blob.remove(blob);
            })
            .collect()
    }

    /// A send to `id` failed with `NoConnection` or `InvalidTarget`: the
    /// route is unusable, so forget it and the next send re-imports. It is
    /// not released: Veilid releases a dead remote route itself
    /// (`ping_validator.rs:610-628`), and releasing an id it already
    /// dropped is an `InvalidArgument` it logs at ERROR (plan C7.9e).
    pub fn invalidate_after_send_failure(&self, id: &RouteId) {
        if let Some(blob) = self.by_id.lock().remove(id) {
            self.by_blob.lock().remove(&blob);
        }
    }
}
