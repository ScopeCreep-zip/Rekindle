//! The Veilid allocator behind `rekindle_route::own_routes::OwnRoutes`, the
//! node's private routes, one owner per route class (plan C7.9a, D4 "own
//! routes react to `RouteChange`"). One per node, like `RouteImports`.
//!
//! veilid-core 0.5.8 (the pinned fork, ADR 0017), checked in source:
//! - `RouteChange.dead_routes` lists routes that died **or were released**
//!   (`veilid_api/types/veilid_state.rs:272-278`), so a dead route is forgotten, never
//!   released: releasing an id Veilid no longer knows is `InvalidArgument`,
//!   logged at ERROR (`veilid_api/api.rs:894-920`).
//! - Allocation returns `TryAgain` until PublicInternet peer info is
//!   published, with too few nodes, or when the new route fails its test
//!   (`route_allocate.rs:170-176,212`, `api.rs:831-839`). Only `TryAgain` is
//!   retried; any other error is a visible failure.
//! - A manual route has no lifetime: Veilid tests it and kills it on a
//!   failed test, and persists it across restarts. So a route that is
//!   replaced while live is released here, or it would live on unused.
//!
//! Allocation is single-flight per class and starts once the node is ready
//! (`public_internet_ready`), so the route exists before login and login
//! is reachable at once (owner, 2026-10-07: login and "Connected" are
//! paired). Logout releases the session's routes and allocates fresh ones
//! for the next login: a route is never shared by two identities, which
//! would link them.

use async_trait::async_trait;
use rekindle_records::port::RouteClass;
use rekindle_route::own_routes::{AllocError, RouteAllocator};

/// The spec of our `Media` route: the configured default hop count (0, as
/// General; `ANONYMITY_HOP_FLOOR` by default, plan RV1), LowLatency,
/// PreferUnordered, so its one ordering is UDP
/// (`safety.rs:24-29`): media frames are `app_message`s that tolerate loss
/// and must not queue behind a retransmit (plan VU decision 2).
pub fn media_private_spec() -> veilid_core::PrivateSpec {
    veilid_core::PrivateSpec {
        // Empty = all available crypto kinds, as the default does.
        crypto_kinds: Vec::new(),
        // 0 = the configured default hop count, as General.
        hop_count: 0,
        stability: veilid_core::Stability::LowLatency,
        sequencing: veilid_core::Sequencing::PreferUnordered,
    }
}

/// The Veilid allocator: `new_private_route` for General (the default
/// spec, PreferOrdered, `safety.rs:16-17`), a LowLatency / PreferUnordered
/// custom route for Media (same hop count, so anonymity is unchanged; only
/// relay selection and its one ordering, UDP, differ).
pub struct VeilidRouteAllocator {
    api: veilid_core::VeilidAPI,
}

impl VeilidRouteAllocator {
    #[must_use]
    pub fn new(api: veilid_core::VeilidAPI) -> Self {
        Self { api }
    }
}

#[async_trait]
impl RouteAllocator for VeilidRouteAllocator {
    type Id = veilid_core::RouteId;

    async fn allocate(&self, class: RouteClass) -> Result<(Self::Id, Vec<u8>), AllocError> {
        let result = match class {
            RouteClass::General => self.api.new_private_route().await,
            RouteClass::Media => {
                self.api
                    .new_custom_private_route(media_private_spec())
                    .await
            }
        };
        match result {
            Ok(route) => Ok((route.route_id, route.blob)),
            Err(veilid_core::VeilidAPIError::TryAgain { message }) => {
                Err(AllocError::TryAgain(message))
            }
            Err(e) => Err(AllocError::Fatal(e.to_string())),
        }
    }

    /// The configured default (`network.rpc.default_route_hop_count`,
    /// which `new_private_route` and a 0-hop spec take, `api.rs:763-857`)
    /// for both classes. `None` once the API is shut down.
    fn hop_count(&self, _class: RouteClass) -> Option<usize> {
        self.api
            .config()
            .ok()
            .map(|config| usize::from(config.network.rpc.default_route_hop_count))
    }

    fn release(&self, id: &Self::Id) {
        if let Err(e) = self.api.release_private_route(id.clone()) {
            tracing::debug!(route = %id, error = %e, "releasing own route failed");
        }
    }
}

#[cfg(test)]
#[path = "own_routes_tests.rs"]
mod tests;
