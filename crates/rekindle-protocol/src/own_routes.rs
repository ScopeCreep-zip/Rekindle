//! The Veilid allocator behind `rekindle_route::own_routes::OwnRoutes`, the
//! node's private routes, one owner per route class (plan C7.9a, D4 "own
//! routes react to `RouteChange`"). One per node, like `RouteImports`.
//!
//! veilid-core 0.5.7, checked in source:
//! - `RouteChange.dead_routes` lists routes that died **or were released**
//!   (`veilid_state.rs:238-245`), so a dead route is forgotten, never
//!   released: releasing an id Veilid no longer knows is `InvalidArgument`,
//!   logged at ERROR (`api.rs:516-531`).
//! - Allocation returns `TryAgain` until PublicInternet peer info is
//!   published, with too few nodes, or when the new route fails its test
//!   (`route_allocate.rs:139-145`, `api.rs:447-457`). Only `TryAgain` is
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
use rekindle_types::config::ANONYMITY_HOP_FLOOR;

/// The spec of our `Sending` route (plan E4.3 RH2, built at RH1r): 3 hops
/// ([`ANONYMITY_HOP_FLOOR`], the depth RH1a gives every inbound route),
/// LowLatency, PreferUnordered. `new_custom_private_route` allocates it
/// `DirectionSet::all()` and marks it published (`api.rs:437-467`), so it
/// serves as a safety route and Veilid retests it
/// (`private_route_management.rs:105-107`); its blob is never handed to a
/// peer. A call pinned to it (`SafetySpec.preferred_route`) takes it first
/// (`route_select.rs:124-152`), and its answer comes back over it
/// (`rpc_processor/destination.rs:477-497`).
pub fn sending_private_spec() -> veilid_core::PrivateSpec {
    veilid_core::PrivateSpec {
        // Empty = all available crypto kinds, as the default does.
        crypto_kinds: Vec::new(),
        hop_count: usize::from(ANONYMITY_HOP_FLOOR),
        stability: veilid_core::Stability::LowLatency,
        sequencing: veilid_core::Sequencing::PreferUnordered,
    }
}

/// The Veilid allocator: `new_private_route` for General, a LowLatency /
/// PreferUnordered custom route for Media (same hop count, so anonymity is
/// unchanged; only relay selection differs), and [`sending_private_spec`]
/// for Sending.
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
                    .new_custom_private_route(veilid_core::PrivateSpec {
                        // Empty = all available crypto kinds, as the default does.
                        crypto_kinds: Vec::new(),
                        // 0 = the configured default hop count, as General.
                        hop_count: 0,
                        stability: veilid_core::Stability::LowLatency,
                        sequencing: veilid_core::Sequencing::PreferUnordered,
                    })
                    .await
            }
            RouteClass::Sending => {
                self.api
                    .new_custom_private_route(sending_private_spec())
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

    fn release(&self, id: &Self::Id) {
        if let Err(e) = self.api.release_private_route(id.clone()) {
            tracing::debug!(route = %id, error = %e, "releasing own route failed");
        }
    }
}
