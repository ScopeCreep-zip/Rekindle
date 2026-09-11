//! Media-class route death handling — the sibling of the general-route
//! heal in [`super::network::handle_route_change`].
//!
//! Voice and video receive over a separate low-latency inbound route
//! (`Stability::LowLatency` + `Sequencing::PreferUnordered`), allocated
//! at login by `login_runtime::allocate_route_with_retry` and re-used by
//! every voice join via `state_helpers::our_media_route_blob`. veilid-core
//! NEVER re-allocates an app route: when it reports one dead in
//! `RouteChange.dead_routes`, re-alloc + re-announce is entirely the app's
//! job. The general route already had that handling; the media route did
//! not, so a dead media route left `our_media_route_blob` handing out a
//! dead blob into every voice join forever. This module closes that gap,
//! mirroring the general path with the media route's own heal-gate.

use std::sync::Arc;
use std::time::Instant;

use crate::state::AppState;
use crate::state_helpers;

/// Media-route sibling of the general-route branch in
/// [`super::network::handle_route_change`].
///
/// When the stored media route ID appears in `change.dead_routes`, this
/// mirrors the general heal exactly, but against the media route's own
/// state and heal-gate:
///
/// 1. **Release + clear** via [`release_media_route`] — defensive: a route
///    Veilid already named dead rejects `release_private_route` with
///    `InvalidArgument`, so the error is logged and ignored, but the call
///    clears both the media route ID and blob so `our_media_route_blob`
///    stops handing out a dead blob. Our own release also echoes back into
///    a later `RouteChange.dead_routes`; the ID is already taken by then,
///    so it cannot re-trigger this heal (and the flap-guard backstops it).
/// 2. **Flap-guard** via the sibling `media_heal_gate` — a flapping network
///    emits `RouteChange` bursts; within the cooldown the heal is skipped
///    (state is already cleared) and the watchdog covers it.
/// 3. **Re-allocate + re-announce** when attached and admitted; otherwise
///    defer (reattach hook + routeless watchdog backstop within ~30s),
///    logging at debug exactly like the general path.
///
/// [`release_media_route`]: rekindle_protocol::routing::RoutingManager::release_media_route
pub(crate) async fn heal_dead_media_route(
    state: &Arc<AppState>,
    change: &veilid_core::VeilidRouteChange,
) {
    let our_media_route_died = {
        let rm = state.routing_manager.read();
        rm.as_ref().is_some_and(|handle| {
            handle
                .manager
                .media_route_id()
                .is_some_and(|our_id| change.dead_routes.contains(&our_id))
        })
    };

    if !our_media_route_died {
        return;
    }

    let heal_admitted = {
        let mut rm = state.routing_manager.write();
        match *rm {
            Some(ref mut handle) => {
                // Defensive release: clears media_route_id + media_route_blob
                // even when the API rejects the already-dead route.
                if let Err(e) = handle.manager.release_media_route() {
                    tracing::debug!(
                        error = %e,
                        "media route release during heal (already dead — expected)"
                    );
                }
                let admitted = handle.media_heal_gate.try_begin(Instant::now());
                // A8 telemetry: admitted/suppressed ratio is the baseline
                // for retuning HEAL_COOLDOWN post-0.5.7.
                if admitted {
                    handle.media_heal_attempts_admitted += 1;
                } else {
                    handle.media_heal_attempts_suppressed += 1;
                }
                admitted
            }
            None => false,
        }
    };

    let attached = state_helpers::is_attached(state);
    if attached && heal_admitted {
        allocate_fresh_media_route(state).await;
    } else {
        // Detached (reattach hook heals) or within the heal cooldown
        // during a flap (watchdog backstops within 30s).
        tracing::debug!(
            attached,
            heal_admitted,
            "dead media route heal deferred (flap guard / detached)"
        );
    }
}

/// Re-allocate the media-class inbound route after Veilid reported the old
/// one dead (or after the watchdog found none). Mirrors
/// [`super::network::allocate_fresh_private_route`] but for the media
/// route: its blob lives on the routing manager (not the node handle) and
/// its only republish surface is the voice re-announce — voice joins read
/// `media_route_blob()` fresh, so there is no DHT profile subkey or mailbox
/// to rewrite, and it never feeds `emit_network_status` (which reflects the
/// general `route_blob` only).
pub(crate) async fn allocate_fresh_media_route(state: &Arc<AppState>) {
    let Some(route_blob) = super::network::new_media_route_with_retry(state, 5).await else {
        tracing::warn!(
            "dead media-route recovery: all allocation attempts failed; watchdog backstop will retry"
        );
        return;
    };

    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            handle
                .manager
                .set_allocated_media_route(route_blob.route_id.clone(), route_blob.blob.clone());
        }
    }

    // Dead-route recovery is the worst case for voice peers — the media
    // blob they hold died with the route. Re-announce immediately so active
    // voice sessions import the fresh blob. (`reannounce_voice_route`
    // resolves the app handle from state internally.) The frontend is not
    // notified: `emit_network_status` reflects the general route only, and
    // nothing user-visible changed here.
    crate::services::voice_adapter::reannounce_voice_route(state);

    tracing::info!(blob_len = route_blob.blob.len(), "re-allocated media route");
}
