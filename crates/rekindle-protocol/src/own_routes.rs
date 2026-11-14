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
use rekindle_types::config::ANONYMITY_HOP_FLOOR;

/// The spec of our `Sending` route (plan E4.3 RH2, built at RH1r): 3 hops
/// ([`ANONYMITY_HOP_FLOOR`], the depth RH1a gives every inbound route),
/// LowLatency, PreferOrdered (plan VU decision 2). `new_custom_private_route`
/// allocates it `DirectionSet::all()` and marks it published
/// (veilid-core 0.5.8 `api.rs:785-857`), so it serves as a safety route and
/// Veilid retests it (`private_route_management.rs:113-120`). We never
/// publish its blob, but every callee receives it: a call pinned to it
/// (`SafetySpec.preferred_route`) takes it first, whatever the call's own
/// sequencing (`route_select.rs:148-180`), and Veilid assembles the same
/// route as the call's `RespondTo` reply route
/// (`rpc_processor/destination.rs:486-528`), so its answer comes back over
/// it. Every peer we call while it lives holds the same blob, so two
/// callees can link their calls to one sender route (threat model L2).
///
/// Why Ordered: Veilid 0.5.8 gives a custom route exactly its spec's
/// preferred ordering (`route_allocate.rs:936` `SequenceOrderingSet::only`,
/// `safety.rs:24-29`), every hop pair proven on that ordering's protocols
/// alone, with no fallback to the other ordering (`route_allocate.rs:99-105`).
/// An Unordered Sending route rode UDP on every hop: on a network that blocks
/// UDP it never allocates, and when its UDP path fails mid-session its
/// questions are lost without an error while the call stays on it (a reply
/// route fixes the safety route, `route_compile.rs:436-447`, and Check #3b
/// does not move it, `:568-569`, `:607-616`) until loopback tests kill it
/// (5 failures, `route_spec_store/mod.rs:51-54`): the lost FriendAccept
/// calls of A/B round 2 (R2-1). Control traffic needs that reliability; the
/// call's first hop follows the route's ordering (`route_compile.rs:106-110`
/// `for_route_orderings`). Media keeps UDP.
pub fn sending_private_spec() -> veilid_core::PrivateSpec {
    veilid_core::PrivateSpec {
        // Empty = all available crypto kinds, as the default does.
        crypto_kinds: Vec::new(),
        hop_count: usize::from(ANONYMITY_HOP_FLOOR),
        stability: veilid_core::Stability::LowLatency,
        sequencing: veilid_core::Sequencing::PreferOrdered,
    }
}

/// The spec of our `Media` route: the configured default hop count (0, as
/// General), LowLatency, PreferUnordered, so its one ordering is UDP
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
/// relay selection and its one ordering, UDP, differ), and
/// [`sending_private_spec`] (PreferOrdered) for Sending.
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

/// An app call from us to our own route `id` (its `blob`), over `context`,
/// whose safety route is that same route: Veilid's loopback (plan RH1r-c
/// O6). A target must be a route in the remote cache
/// (`rpc_processor/destination.rs:328-366`), so our own blob is imported
/// for the call through the one importer ([`RouteImports::loopback_import`]);
/// Veilid accepts it (`route_remote.rs:28-53` checks only the first hop).
/// The import shares the route's id (both hash the route's keys,
/// `route_remote.rs:191-221`, `route_allocate.rs:971-990`) and a release
/// prefers the remote entry (`route_spec_store/mod.rs:227-233`), so an
/// import this call made is released at once after it, whatever the
/// outcome: otherwise our later release of the route would drop the import
/// and leak the allocation.
///
/// # Errors
/// The import or the call failed (a timeout is a failed round trip).
pub async fn loopback_call(
    api: &veilid_core::VeilidAPI,
    imports: &crate::dht::route_imports::RouteImports,
    context: &veilid_core::RoutingContext,
    blob: &[u8],
    message: Vec<u8>,
) -> Result<Vec<u8>, rekindle_records::port::PortError> {
    use rekindle_records::port::PortError;
    let (imported, owned) = imports
        .loopback_import(blob)
        .map_err(|e| PortError::Routing(e.to_string()))?;
    let answer = context
        .app_call(veilid_core::Target::RouteId(imported.clone()), message)
        .await
        .map_err(|e| PortError::Send(e.to_string()));
    if owned {
        if let Err(e) = api.release_private_route(imported) {
            tracing::debug!(error = %e, "releasing the own route import failed");
        }
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilid_core::{SequenceOrdering, Sequencing};

    /// Plan VU decision 2: Sending carries Ordered (TCP/WS), Media
    /// Unordered (UDP); each custom route carries only its preference's
    /// ordering (`route_allocate.rs:936`).
    #[test]
    fn sending_is_ordered_and_media_is_unordered() {
        let sending = sending_private_spec();
        assert_eq!(sending.sequencing, Sequencing::PreferOrdered);
        assert_eq!(
            sending.sequencing.preferred_ordering(),
            SequenceOrdering::Ordered
        );
        assert_eq!(sending.hop_count, usize::from(ANONYMITY_HOP_FLOOR));
        let media = media_private_spec();
        assert_eq!(media.sequencing, Sequencing::PreferUnordered);
        assert_eq!(
            media.sequencing.preferred_ordering(),
            SequenceOrdering::Unordered
        );
    }
}
