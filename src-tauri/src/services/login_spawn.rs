//! Phase 23.D.4 — `spawn_login_services` extracted from
//! `login_runtime.rs` to keep that file under the 500-LoC cap.
//! Spawns the post-login background-service set: governance hydration,
//! presence poll + keepalive, event reminders, sync loop, DHT publish,
//! route refresh, idle service, presence heartbeat. Each runs on the
//! login scope, so ending the session stops it (plan C4).

use std::sync::Arc;

use rekindle_lifecycle::{ScopeClosed, SessionScope};

use crate::services;
use crate::state::SharedState;
use rekindle_db::Db;

use super::login_runtime::{spawn_dht_publish, DhtKeysConfig};

/// Background task: start sync service and DHT publish using the existing node.
///
/// The Veilid node and dispatch loop are already running (started at app startup).
/// This function only spawns user-specific services: sync and DHT publish.
///
/// # Errors
/// [`ScopeClosed`] when the session ended while its services were starting.
pub(super) fn spawn_login_services(
    app: &tauri::AppHandle,
    state: &SharedState,
    scope: &Arc<SessionScope>,
    pool: Db,
    prekey_bundle_bytes: Vec<u8>,
    dht_keys: DhtKeysConfig,
) -> Result<(), ScopeClosed> {
    // Check that the node is running (should be — started at app startup)
    let node_alive = state.node.read().is_some();
    if !node_alive {
        tracing::error!("Veilid node not running at login — background services cannot start");
        return Ok(());
    }

    // W14.4 — voice drop telemetry: 1s tick that emits
    // VoiceEvent::PacketsDropped if any packets were dropped since the
    // last tick. Lives at login scope so it's available even before the
    // first call session starts.
    crate::services::voice_adapter::spawn_drop_telemetry(state, app, scope)?;

    // Pre-set existing DHT keys from SQLite on NodeHandle
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            if let Some(ref dht_key) = dht_keys.existing_dht_key {
                nh.profile_dht_key = Some(dht_key.clone());
            }
            if let Some(ref fl_key) = dht_keys.existing_friend_list_key {
                nh.friend_list_dht_key = Some(fl_key.clone());
            }
        }
    }

    // ── Phase 1-3: Open DHT records + hydrate + rebuild governance ──
    // These involve slow DHT network reads that can take 30-60+ seconds.
    // Run them in the background so login returns immediately with SQLite data.
    // The frontend can show channels/roles/members from SQLite right away;
    // background tasks will update state when DHT reads complete.
    {
        let bg_app = app.clone();
        let bg_state = Arc::clone(state);
        scope.spawn_with_token("community DHT hydration", |stop| async move {
            // Wait (bounded) for the routing table to mature before opening the
            // community DHT records — on a sparse cold-start table the opens hit
            // transient KeyNotFound and the channel watch fails "record not
            // open". On timeout, proceed best-effort anyway (the watch-retry +
            // keepalive recover residuals); don't skip hydration entirely, which
            // would leave communities unsynced on a slow network.
            let ready = stop
                .run_until_cancelled(super::login_runtime::wait_for_network_ready(
                    bg_state.network_ready_rx.clone(),
                    60,
                ))
                .await;
            let Some(ready) = ready else {
                return;
            };
            if !ready {
                tracing::warn!(
                    "public internet not ready within 60s — running community DHT hydration best-effort"
                );
            }
            // Hydration's DHT work is record-pool calls: at logout the pool's
            // drain releases a step waiting on one (plan C7.6g), and each
            // step checks stop between communities.
            crate::services::governance_adapter::open_community_dht_records(&bg_state).await;
            crate::services::governance_adapter::hydrate_community_state_from_dht(&bg_state).await;
            crate::services::governance_adapter::rebuild_governance_from_dht(&bg_state).await;
            crate::services::governance_adapter::republish_active_records(&bg_state).await;
            if stop.is_cancelled() {
                return;
            }
            tracing::info!("background DHT hydration complete — governance state rebuilt");

            // Emit GovernanceUpdated for each community so the frontend refreshes
            let community_ids: Vec<String> = bg_state.communities.read().keys().cloned().collect();
            for cid in &community_ids {
                crate::event_dispatch::emit_subscription(
                    &bg_app,
                    &rekindle_types::subscription_events::SubscriptionEvent::Governance(
                        rekindle_types::subscription_events::GovernanceEvent::GovernanceRebuilt {
                            community: cid.clone(),
                        },
                    ),
                );
            }
            // Also emit MembersRefreshed so the frontend re-fetches members
            // even if the presence poll hasn't completed its first tick yet.
            for cid in &community_ids {
                crate::event_dispatch::emit_membership(
                    &bg_app,
                    rekindle_types::subscription_events::MembershipEvent::MembersRefreshed {
                        community: cid.clone(),
                    },
                );
            }
        })?;
    }

    // ── Phase 4: History catch-up ──
    // Each community's presence poll, keepalive and inspect loop start when
    // hydration hands its records to the host (`leases::records_ready`).
    {
        let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
        for community_id in community_ids {
            // Mutual Aid §14.2: returning members request missing message
            // ranges from peers who advertise them. The 15-second delay
            // inside the helper lets the presence poll populate
            // `history_ranges` first.
            services::community::join::schedule_history_catchup(Arc::clone(state), community_id);
        }
    }

    // Peer-reliability counters are flushed to SQLite every 30 s.
    services::community::start_peer_reliability_flush(Arc::clone(state), pool.clone());

    // Held channel writes resolve their messages' delivery when they settle
    // (plan C7.13).
    services::community::start_delivery_settle(Arc::clone(state), scope)?;

    // ── Phase 5: Start local event reminder scheduler ──
    services::community::start_event_reminders(Arc::clone(state), pool.clone(), scope)?;

    // ── Phase 6: Start sync service (first tick at 10s — after election settles) ──
    let sync_state = Arc::clone(state);
    let sync_pool = pool.clone();
    let sync_app = app.clone();
    scope.spawn_with_token("sync service", |stop| {
        services::sync_service::start_sync_loop(sync_state, sync_pool, sync_app, stop)
    })?;

    // ── Phase 7: Start background services (non-critical, can run concurrently) ──

    // DHT publish (profile + prekeys)
    let publish_app = app.clone();
    let publish_state = Arc::clone(state);
    scope.spawn_with_token("DHT publish", |stop| {
        spawn_dht_publish(
            publish_app,
            publish_state,
            pool,
            prekey_bundle_bytes,
            dht_keys,
            stop,
        )
    })?;

    // Our routes' republisher: a new blob is rewritten wherever peers read
    // it (plan C7.9b).
    let publish_app = app.clone();
    let publish_state = Arc::clone(state);
    scope.spawn_with_token("route republisher", |stop| {
        services::veilid::route_publish::run(publish_app, publish_state, stop)
    })?;

    // Route watchdog: peer-route eviction, and a backstop want for a route
    // whose allocation failed for good.
    let route_watchdog_state = Arc::clone(state);
    scope.spawn_with_token("route watchdog", |stop| {
        services::veilid::route_watchdog_loop(route_watchdog_state, stop)
    })?;

    // Idle/auto-away service
    let idle_app = app.clone();
    let idle_state = Arc::clone(state);
    scope.spawn_with_token("idle service", |stop| {
        services::idle_service::run_idle_service(idle_app, idle_state, stop)
    })?;

    // The session's one STATUS publisher: status changes and the 60 s
    // heartbeat (plan C7.8c).
    let publisher_state = Arc::clone(state);
    scope.spawn_with_token("status publisher", |stop| {
        services::presence_service::run_status_publisher(publisher_state, stop)
    })?;
    Ok(())
}
