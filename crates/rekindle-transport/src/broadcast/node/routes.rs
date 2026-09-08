//! Route allocation methods and the event-driven route authority loop —
//! the sole owner of the personal route and (operator) community mailbox
//! routes.

use std::sync::Arc;

use tokio::sync::mpsc;
use veilid_core::VeilidAPI;

use super::updates::personal_route_died;
use super::{build_routing_context, TransportNode};
use crate::broadcast::dht::DhtStore;
use crate::broadcast::peer_registry::PeerTarget;
use crate::broadcast::peer_route::RouteManager;
use crate::config::TransportConfig;
use crate::error::{Result, TransportError};
use crate::shared::SharedState;

/// Default maximum wait time for route allocation (seconds).
/// 30 minutes — generous for edge/far-edge nodes with intermittent connectivity.
/// Override via `allocate_route_with_deadline()` for specific use cases.
/// Pass 0 for unlimited retry (beacon mode).
const DEFAULT_ROUTE_ALLOC_MAX_SECS: u64 = 1800;

impl TransportNode {
    /// Allocate a private route, retrying with exponential backoff until
    /// the network is ready or the deadline expires.
    ///
    /// Veilid returns `TryAgain` when the network hasn't reached a state
    /// where routes can be allocated (e.g., no PublicInternet network class).
    /// This is normal during startup — the Veilid node needs time to discover
    /// peers, establish NAT mappings, and confirm reachability.
    ///
    /// Retry strategy:
    /// - Initial backoff: 500ms (fast first retries for quick connections)
    /// - Exponential growth: 500ms → 1s → 2s → 4s → 8s → 15s (ceiling)
    /// - Ceiling: 15 seconds (prevents excessive gaps between attempts)
    /// - Normal deadline: 90 seconds (covers ~90% of network conditions)
    /// - After normal deadline: log advisory, continue retrying at ceiling
    /// - Maximum deadline: configurable, default 30 minutes (edge/far-edge)
    /// - Non-TryAgain errors: fail immediately (not a network readiness issue)
    ///
    /// The `max_wait_secs` parameter controls the hard deadline. Pass 0 for
    /// unlimited retry (beacon mode for intermittently connected nodes).
    pub async fn allocate_route(&self) -> Result<(String, Vec<u8>)> {
        self.allocate_route_with_deadline(DEFAULT_ROUTE_ALLOC_MAX_SECS)
            .await
    }

    /// Allocate a private route with a configurable deadline in seconds.
    /// Pass 0 for unlimited retry (beacon mode).
    pub async fn allocate_route_with_deadline(
        &self,
        max_wait_secs: u64,
    ) -> Result<(String, Vec<u8>)> {
        let start = std::time::Instant::now();
        let mut backoff = std::time::Duration::from_millis(500);
        let ceiling = std::time::Duration::from_secs(15);
        let normal_deadline = std::time::Duration::from_secs(90);
        let hard_deadline = if max_wait_secs == 0 {
            std::time::Duration::from_secs(u64::MAX) // effectively unlimited
        } else {
            std::time::Duration::from_secs(max_wait_secs)
        };
        let mut warned_slow = false;
        let mut attempt = 0u32;

        loop {
            attempt += 1;
            match self.api.new_private_route().await {
                Ok(rb) => {
                    let id_str = rb.route_id.to_string();
                    let blob = rb.blob.clone();
                    self.route_manager.write().set_route(rb.route_id, rb.blob);
                    if attempt > 1 {
                        tracing::info!(
                            attempt,
                            elapsed_secs = start.elapsed().as_secs(),
                            "route allocated after retry"
                        );
                    }
                    return Ok((id_str, blob));
                }
                Err(e) => {
                    let msg = e.to_string();

                    // Non-retryable errors fail immediately.
                    if !msg.contains("TryAgain") {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!("{e}"),
                        });
                    }

                    let elapsed = start.elapsed();

                    // Hard deadline exceeded.
                    if elapsed >= hard_deadline {
                        return Err(TransportError::RouteAllocationFailed {
                            reason: format!(
                                "network not ready after {} attempts over {}s — \
                                 check network connectivity and Veilid bootstrap peers",
                                attempt,
                                elapsed.as_secs()
                            ),
                        });
                    }

                    // Advisory after normal deadline — operation is taking longer than expected.
                    if !warned_slow && elapsed >= normal_deadline {
                        warned_slow = true;
                        tracing::warn!(
                            attempt,
                            elapsed_secs = elapsed.as_secs(),
                            next_retry_secs = ceiling.as_secs(),
                            "route allocation taking longer than expected — \
                             network may be slow to attach, retrying at {}s intervals",
                            ceiling.as_secs()
                        );
                    }

                    tracing::debug!(
                        attempt,
                        elapsed_secs = elapsed.as_secs(),
                        backoff_ms = backoff.as_millis(),
                        "network not ready for route allocation, retrying"
                    );

                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(ceiling);
                }
            }
        }
    }

    pub fn import_route(&self, route_blob: &[u8]) -> Result<PeerTarget> {
        let route_id = self
            .api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| TransportError::RouteImportFailed {
                peer: String::new(),
                reason: format!("{e}"),
            })?;
        Ok(PeerTarget { route_id })
    }
}

/// Event-driven route authority — the ONLY owner of this transport's
/// personal route and (operator) community mailbox routes.
///
/// Routes are never rotated on a timer: veilid-core tests route health
/// itself and reports death via `RouteChange`. Death events arrive on
/// `heal_rx` (fed by the dispatch loop); the heal is forget → allocate
/// → republish. The watchdog tick only repairs "attached but
/// routeless" states (failed heals, startup races) and releases routes
/// for communities we stopped operating. Explicit Veilid release
/// happens only at shutdown / operator-exit — dead routes are
/// forgotten, never released (release on a dead route is an
/// "Invalid argument" API error).
pub(crate) enum RouteAuthorityEvent {
    /// Veilid reported these local routes dead (`RouteChange`).
    DeadLocalRoutes(Vec<veilid_core::RouteId>),
}

/// Allocate + install + republish the personal route (profile subkey
/// `PROFILE_SUBKEY_ROUTE_BLOB` + personal mailbox). Publish failures
/// are logged, not fatal — the blob is installed locally and the
/// records self-heal on the next publish trigger.
async fn heal_personal_route(
    api: &VeilidAPI,
    route_manager: &Arc<parking_lot::RwLock<RouteManager>>,
    session: &Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    config: &Arc<TransportConfig>,
) {
    let rb = match api.new_private_route().await {
        Ok(rb) => rb,
        Err(e) => {
            tracing::warn!(error = %e, "personal route heal: allocation failed — watchdog retries");
            return;
        }
    };
    route_manager
        .write()
        .set_route(rb.route_id, rb.blob.clone());
    tracing::info!("personal route healed (event-driven)");

    let (profile_key, mailbox_key) = {
        let guard = session.read();
        match guard.as_ref() {
            Some(s) => (
                s.identity.profile_dht_key.clone(),
                s.identity.mailbox_dht_key.clone(),
            ),
            None => return, // no session yet — resume() publishes on login
        }
    };
    let rc = match build_routing_context(api, &config.safety.dht) {
        Ok(rc) => rc,
        Err(e) => {
            tracing::warn!(error = %e, "personal route heal: no routing context for republish");
            return;
        }
    };
    if !profile_key.is_empty() {
        if let Err(e) = crate::broadcast::dht::record::set(
            &rc,
            &profile_key,
            crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
            rb.blob.clone(),
            None,
        )
        .await
        {
            tracing::warn!(error = %e, "personal route heal: profile republish failed");
        }
    }
    if !mailbox_key.is_empty() {
        let dht = DhtStore::new(rc);
        if let Err(e) = dht.mailbox().update_route(&mailbox_key, &rb.blob).await {
            tracing::warn!(error = %e, "personal route heal: mailbox republish failed");
        }
    }
}

pub(super) async fn run_route_authority_loop(
    api: VeilidAPI,
    route_manager: Arc<parking_lot::RwLock<RouteManager>>,
    watchdog_secs: u64,
    mut heal_rx: mpsc::Receiver<RouteAuthorityEvent>,
    mut shutdown_rx: mpsc::Receiver<()>,
    session: Arc<parking_lot::RwLock<Option<crate::session::Session>>>,
    config: Arc<TransportConfig>,
    shared: Arc<SharedState>,
) {
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(watchdog_secs.max(1)));
    interval.tick().await; // skip immediate first tick

    // The personal route is the only one this loop owns. Community
    // mailbox routes are gone with the coordinator: a member is reached
    // through its own registry row, so there is no shared endpoint for
    // one peer to keep alive on everyone's behalf.
    let mut heal_gate =
        rekindle_route::lifecycle::HealGate::new(rekindle_route::lifecycle::HEAL_COOLDOWN);

    loop {
        tokio::select! {
            Some(event) = heal_rx.recv() => {
                // Coalesce a burst of RouteChange events into one heal.
                let RouteAuthorityEvent::DeadLocalRoutes(mut dead) = event;
                while let Ok(RouteAuthorityEvent::DeadLocalRoutes(more)) = heal_rx.try_recv() {
                    dead.extend(more);
                }
                let personal_died =
                    personal_route_died(&dead, route_manager.read().route_id());
                if personal_died {
                    route_manager.write().forget_route();
                } else {
                    continue;
                }
                if !shared.is_attached() {
                    // Detached flap: state is forgotten; reattach +
                    // watchdog heal once the network is back.
                    tracing::debug!("dead route(s) while detached — heal deferred to watchdog");
                    continue;
                }
                let admitted = heal_gate.try_begin(std::time::Instant::now());
                // A8 telemetry: admitted/suppressed ratio is the baseline
                // for retuning HEAL_COOLDOWN post-0.5.7 (upstream now
                // damps relay-switch route churn itself).
                shared.count_heal_attempt(admitted);
                if !admitted {
                    tracing::debug!("dead-route heal within cooldown — watchdog backstops");
                    continue;
                }
                heal_personal_route(&api, &route_manager, &session, &config).await;
            }
            _ = interval.tick() => {
                if !shared.is_attached() {
                    continue;
                }
                // Routeless watchdog: heal anything that should exist
                // but doesn't.
                if !route_manager.read().has_route() {
                    heal_personal_route(&api, &route_manager, &session, &config).await;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("route authority loop shutting down");
                break;
            }
        }
    }
}
