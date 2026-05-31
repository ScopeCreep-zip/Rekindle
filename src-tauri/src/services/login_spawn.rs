//! Phase 23.D.4 — `spawn_login_services` extracted from
//! `login_runtime.rs` to keep that file under the 500-LoC cap.
//! Spawns the post-login background-service set: governance hydration,
//! presence poll + keepalive, event reminders, sync loop, DHT publish,
//! route refresh, idle service, presence heartbeat. Stores handles on
//! `state.background_handles` so logout can abort them.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::db::DbPool;
use crate::services;
use crate::state::SharedState;

use super::login_runtime::{spawn_dht_publish, DhtKeysConfig};

/// Background task: start sync service and DHT publish using the existing node.
///
/// The Veilid node and dispatch loop are already running (started at app startup).
/// This function only spawns user-specific services: sync and DHT publish.
pub(super) fn spawn_login_services(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: DbPool,
    prekey_bundle_bytes: Option<Vec<u8>>,
    dht_keys: DhtKeysConfig,
) {
    // Check that the node is running (should be — started at app startup)
    let node_alive = state.node.read().is_some();
    if !node_alive {
        tracing::error!("Veilid node not running at login — background services cannot start");
        return;
    }

    // W14.4 — voice drop telemetry: 1s tick that emits
    // VoiceEvent::PacketsDropped if any packets were dropped since the
    // last tick. Lives at login scope so it's available even before the
    // first call session starts.
    crate::services::voice_adapter::spawn_drop_telemetry(state, app);

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
        tokio::spawn(async move {
            crate::services::governance_adapter::open_community_dht_records(&bg_state).await;
            crate::services::governance_adapter::hydrate_community_state_from_dht(&bg_state).await;
            crate::services::governance_adapter::rebuild_governance_from_dht(&bg_state).await;
            tracing::info!("background DHT hydration complete — governance state rebuilt");

            // Emit GovernanceUpdated for each community so the frontend refreshes
            let community_ids: Vec<String> = bg_state.communities.read().keys().cloned().collect();
            for cid in &community_ids {
                crate::event_dispatch::emit_live(
                    &bg_app,
                    "community-event",
                    &crate::channels::CommunityEvent::GovernanceUpdated {
                        community_id: cid.clone(),
                    },
                );
            }
            // Also emit MembersRefreshed so the frontend re-fetches members
            // even if the presence poll hasn't completed its first tick yet.
            for cid in &community_ids {
                crate::event_dispatch::emit_live(
                    &bg_app,
                    "community-event",
                    &crate::channels::CommunityEvent::MembersRefreshed {
                        community_id: cid.clone(),
                    },
                );
            }
        });
    }

    // ── Phase 4: Start presence poll + DHT keepalive ──
    {
        let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
        for community_id in community_ids {
            services::community::start_presence_poll(state, community_id.clone());
            services::community::start_dht_keepalive(Arc::clone(state), community_id.clone());
            // Mutual Aid §14.2: returning members request missing message
            // ranges from peers who advertise them. The 15-second delay
            // inside the helper lets the presence poll populate
            // `history_ranges` first.
            services::community::join::schedule_history_catchup(
                Arc::clone(state),
                community_id,
            );
        }
    }

    // ── Phase 5: Start local event reminder scheduler ──
    let reminder_handle =
        services::community::start_event_reminders(Arc::clone(state), pool.clone());

    // ── Phase 6: Start sync service (first tick at 10s — after election settles) ──
    let (sync_shutdown_tx, sync_shutdown_rx) = mpsc::channel::<()>(1);
    let sync_state = Arc::clone(state);
    let sync_pool = pool.clone();
    let sync_app = app.clone();
    let sync_handle = tauri::async_runtime::spawn(async move {
        services::sync_service::start_sync_loop(sync_state, sync_pool, sync_app, sync_shutdown_rx)
            .await;
    });
    *state.sync_shutdown_tx.write() = Some(sync_shutdown_tx);

    // ── Phase 7: Start background services (non-critical, can run concurrently) ──

    // DHT publish (profile + prekeys)
    let dht_handle = tauri::async_runtime::spawn(spawn_dht_publish(
        app.clone(),
        state.clone(),
        pool,
        prekey_bundle_bytes,
        dht_keys,
    ));

    // Proactive route refresh loop (re-allocates our private route every 120s)
    let (route_refresh_shutdown_tx, route_refresh_shutdown_rx) = mpsc::channel::<()>(1);
    let route_refresh_app = app.clone();
    let route_refresh_state = Arc::clone(state);
    let route_refresh_handle = tauri::async_runtime::spawn(services::veilid::route_refresh_loop(
        route_refresh_app,
        route_refresh_state,
        route_refresh_shutdown_rx,
    ));
    *state.route_refresh_shutdown_tx.write() = Some(route_refresh_shutdown_tx);

    // Idle/auto-away service
    let idle_tx = services::idle_service::start_idle_service(app.clone(), Arc::clone(state));
    *state.idle_shutdown_tx.write() = Some(idle_tx);

    // Presence heartbeat loop (re-publishes status with fresh timestamp every 120s)
    let (heartbeat_tx, heartbeat_rx) = mpsc::channel::<()>(1);
    let heartbeat_state = Arc::clone(state);
    let heartbeat_handle = tauri::async_runtime::spawn(
        services::presence_service::start_heartbeat_loop(heartbeat_state, heartbeat_rx),
    );
    *state.heartbeat_shutdown_tx.write() = Some(heartbeat_tx);

    // Store sub-task handles so they can be aborted on logout
    {
        let mut handles = state.background_handles.lock();
        handles.push(reminder_handle);
        handles.push(sync_handle);
        handles.push(dht_handle);
        handles.push(route_refresh_handle);
        handles.push(heartbeat_handle);
    }
}
