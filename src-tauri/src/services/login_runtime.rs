//! Phase 23.C — Tauri-runtime spawn-and-wire orchestration.
//!
//! Pre-Phase-23, every post-login background-service spawn lived
//! inline in `commands/auth.rs`. Per Invariant 7, `commands/` should
//! hold THIN handlers (≤20 LoC) that delegate to adapters and crates;
//! the actual spawn-and-wire ceremony for game detection, sync,
//! DHT publish, route refresh, idle, heartbeat, etc. is legitimate
//! Tauri-runtime glue and lives here.
//!
//! `start_background_services` is the entry point auth's
//! `create_identity` / `login` commands call. Everything else here
//! is private orchestration delegated to that function.
//!
//! NB this module is NOT business logic — it contains zero CRDT
//! merges, sig verifies, persistence, or protocol-level decisions.
//! All such logic was already chiral-split into rekindle-* crates
//! during earlier 23.C wakes. This module is purely the wiring +
//! tokio::spawn glue that connects existing crate-side primitives
//! to AppState's shutdown channels and background-handle Vec.

use std::sync::Arc;

use tauri::Manager as _;
use tokio::sync::mpsc;

use crate::db::DbPool;
use crate::services;
use crate::state::{SharedState, SignalManagerHandle};

/// Stored DHT keys and owner keypairs loaded from `SQLite` during login.
///
/// Passed through to background services so they can reuse existing DHT records
/// instead of creating new ones on every login.
pub struct DhtKeysConfig {
    pub existing_dht_key: Option<String>,
    pub existing_friend_list_key: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub friend_list_owner_keypair: Option<String>,
    pub account_dht_key: Option<String>,
    pub account_owner_keypair: Option<String>,
    pub mailbox_dht_key: Option<String>,
}

/// Initialize Signal encryption and spawn all background services (non-blocking).
///
/// Returns immediately after setting up in-memory state. Uses the already-running
/// Veilid node (started at app launch) for DHT publishing, sync, and messaging.
/// Game detection and sync services are spawned as background tasks so login
/// returns near-instantly to the frontend.
pub fn start_background_services(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    secret_key: &[u8; 32],
    dht_keys: DhtKeysConfig,
) {
    // Initialize Signal Protocol session manager (returns serialized PreKeyBundle)
    let prekey_bundle_bytes = initialize_signal_manager(app, state, secret_key);

    // Clear any stale background handles from a previous session
    state.background_handles.lock().clear();

    // Start game detection (only after login — avoids burning CPU before auth)
    let (game_shutdown_tx, game_shutdown_rx) = mpsc::channel::<()>(1);
    services::game_service::initialize(state, game_shutdown_tx);
    let game_app = app.clone();
    let game_state = Arc::clone(state);
    let game_pool = pool.clone();
    let game_handle = tauri::async_runtime::spawn(async move {
        services::game_service::start_game_detection(
            game_app,
            game_state,
            game_pool,
            game_shutdown_rx,
        )
        .await;
    });

    // Store the game handle so logout can abort it
    state.background_handles.lock().push(game_handle);

    // The Veilid node is already running (started at app startup).
    // Just spawn sync + DHT publish as background tasks.
    super::login_spawn::spawn_login_services(
        app,
        state,
        pool.clone(),
        prekey_bundle_bytes,
        dht_keys,
    );
}

/// Allocate a Veilid private route with retry.
///
/// Route allocation can fail transiently after the network becomes ready because
/// peerinfo may not have been published yet. We retry up to `max_attempts` times
/// with a 3-second delay between attempts.
async fn allocate_route_with_retry(
    app_handle: &tauri::AppHandle,
    state: &SharedState,
    max_attempts: u32,
) -> Option<Vec<u8>> {
    for attempt in 1..=max_attempts {
        let api = crate::state_helpers::veilid_api(state)?;

        match api.new_private_route().await {
            Ok(route_blob) => {
                // Store on routing manager
                {
                    let mut rm = state.routing_manager.write();
                    if let Some(ref mut handle) = *rm {
                        handle.manager.set_allocated_route(
                            route_blob.route_id.clone(),
                            route_blob.blob.clone(),
                        );
                    }
                }
                // Store on node handle
                if let Some(ref mut nh) = *state.node.write() {
                    nh.route_blob = Some(route_blob.blob.clone());
                }
                // Notify the frontend immediately about the new route
                services::veilid::emit_network_status(app_handle, state);
                tracing::info!(
                    attempt,
                    blob_len = route_blob.blob.len(),
                    route_count = route_blob.blob.first().copied().unwrap_or(0),
                    "private route allocated"
                );
                return Some(route_blob.blob);
            }
            Err(e) => {
                tracing::warn!(attempt, max_attempts, error = %e, "route allocation attempt failed");
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    }
    None
}

/// Wait for public internet readiness, allocate a private route, then publish
/// profile and friend list to DHT.
pub(super) async fn spawn_dht_publish(
    app_handle: tauri::AppHandle,
    state: SharedState,
    pool: DbPool,
    prekey_bundle_bytes: Option<Vec<u8>>,
    dht_keys: DhtKeysConfig,
) {
    // Wait for public internet ready via watch channel
    let mut rx = state.network_ready_rx.clone();
    let ready = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if *rx.borrow_and_update() {
                return true;
            }
            if rx.changed().await.is_err() {
                return false; // channel closed
            }
        }
    })
    .await
    .unwrap_or(false);

    if !ready {
        tracing::warn!(
            "timed out waiting for public internet readiness (60s) — \
             DHT publish deferred to sync loop"
        );
        return;
    }

    // Brief delay to let Veilid publish peerinfo — route assembly requires
    // peerinfo to be published, which happens shortly after public_internet_ready.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Allocate private route now that the network is ready (with retry).
    // 15 attempts × 3s delay = up to 45s window for peerinfo publication.
    let route_blob = allocate_route_with_retry(&app_handle, &state, 15).await;
    if route_blob.is_none() {
        tracing::warn!(
            "failed to allocate private route after retries — peers won't be able to message us"
        );
    }

    // Route is now available — trigger immediate presence re-writes for all
    // communities so peers can discover our route_blob in the SMPL registry.
    if route_blob.is_some() {
        let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();

        // Reset needs_initial_sync so PresenceUpdate re-broadcasts with real route
        {
            let mut communities = state.communities.write();
            for cid in &community_ids {
                if let Some(cs) = communities.get_mut(cid) {
                    if let Some(ref mut g) = cs.gossip {
                        g.needs_initial_sync = true;
                    }
                }
            }
        }

        // Trigger immediate presence poll for each community
        for cid in community_ids {
            let poll_state = state.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    services::community::presence_poll_tick_public(&poll_state, &cid).await
                {
                    tracing::debug!(
                        community = %cid,
                        error = %e,
                        "route-ready presence poll failed"
                    );
                }
            });
        }
        tracing::info!(
            "route allocated — triggered immediate presence re-write for all communities"
        );
    }

    // Create or open mailbox DHT record
    if let Err(e) = services::dht_publish_service::publish_mailbox(
        &state,
        &pool,
        dht_keys.mailbox_dht_key.as_ref(),
        route_blob.as_deref(),
    )
    .await
    {
        tracing::warn!(error = %e, "mailbox publish failed");
    }

    tracing::info!("public internet ready — publishing profile to DHT");

    if let Err(e) = services::dht_publish_service::publish_profile(
        &state,
        &pool,
        prekey_bundle_bytes,
        dht_keys.existing_dht_key,
        dht_keys.dht_owner_keypair,
    )
    .await
    {
        tracing::warn!(error = %e, "DHT profile publish failed — will retry on next sync");
    }

    if let Err(e) = services::dht_publish_service::publish_friend_list(
        &state,
        &pool,
        dht_keys.existing_friend_list_key,
        dht_keys.friend_list_owner_keypair,
    )
    .await
    {
        tracing::warn!(error = %e, "DHT friend list publish failed — will retry on next sync");
    }

    // Immediate friend sync now that network is up
    if let Err(e) = services::sync_service::sync_friends_now(&state, &app_handle).await {
        tracing::warn!(error = %e, "immediate friend sync failed");
    }

    // Publish account record (Phase 3)
    if let Err(e) = services::dht_publish_service::publish_account(
        &state,
        &pool,
        dht_keys.account_dht_key,
        dht_keys.account_owner_keypair,
    )
    .await
    {
        tracing::warn!(error = %e, "DHT account publish failed — will retry on next sync");
    }
}

/// Initialize the Signal Protocol session manager with the identity key.
///
/// Creates in-memory stores for identity, prekeys, and sessions, then
/// generates an initial `PreKeyBundle` for DHT publication.
///
/// Returns the serialized `PreKeyBundle` bytes if generation succeeded,
/// so the caller can publish them to DHT profile subkey 5.
fn initialize_signal_manager(
    app: &tauri::AppHandle,
    state: &SharedState,
    secret_key: &[u8; 32],
) -> Option<Vec<u8>> {
    use rekindle_crypto::signal::SignalSessionManager;

    // Phase 3b — Signal identity store holds the Ed25519 keypair bytes.
    // PQXDH derives X25519 from these internally via `to_scalar_bytes`
    // matching `Identity::to_x25519_secret`. Storing X25519 bytes would
    // double-derive and produce mismatched DH outputs; it would also
    // break bundle signature verification because the published
    // `identity_key` is what the peer feeds into `VerifyingKey::from_bytes`
    // for SPK/PQ signature checks.
    let identity = rekindle_crypto::Identity::from_secret_bytes(secret_key);
    let identity_private = identity.secret_key_bytes().to_vec();
    let identity_public = identity.public_key_bytes().to_vec();

    // Registration ID — derive deterministically from the public key so it's stable
    let pub_bytes = identity.public_key_bytes();
    let registration_id =
        u32::from_le_bytes([pub_bytes[0], pub_bytes[1], pub_bytes[2], pub_bytes[3]]);

    // B7/D4 (P0.1+P0.5+P1.2) — Stronghold-backed Signal stores. Without
    // this, every restart wiped Memory*Stores and friends had to re-handshake
    // — a social-engineering opportunity for vulnerable users (an attacker
    // who can prompt a re-handshake can substitute their own key). The
    // Stronghold-backed wrappers prime their in-memory cache from disk on
    // construction and write-through to Stronghold on every store, so the
    // session graph survives restart and corruption is recoverable rather
    // than the default state.
    let keystore_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app.state();
    let identity_store = crate::signal_stores::StrongholdIdentityStore::new(
        keystore_handle.inner().clone(),
        identity_private,
        identity_public,
        registration_id,
    );
    let prekey_store =
        crate::signal_stores::StrongholdPreKeyStore::new(keystore_handle.inner().clone());
    let session_store =
        crate::signal_stores::StrongholdSessionStore::new(keystore_handle.inner().clone());

    let manager = SignalSessionManager::new(
        Box::new(identity_store),
        Box::new(prekey_store),
        Box::new(session_store),
    );

    // P1.2 — prefer the existing prekey bundle if Stronghold already
    // has prekey #1 + signed_prekey #1 from a prior login. Calling
    // `generate_prekey_bundle` unconditionally would overwrite both
    // keys in Stronghold AND publish a fresh bundle to DHT subkey 5,
    // breaking peers' cached PreKeyBundles + any in-flight messages
    // encrypted under the previous bundle. Mint fresh ONLY when no
    // existing bundle is loadable.
    let bundle_result = match manager.load_existing_prekey_bundle(1, Some(1), Some(1)) {
        Ok(Some(bundle)) => {
            tracing::info!(
                registration_id = bundle.registration_id,
                "Signal session manager initialized — reusing existing PreKeyBundle from Stronghold"
            );
            Ok(bundle)
        }
        Ok(None) => {
            tracing::info!("No existing PreKeyBundle in Stronghold — generating fresh bundle");
            manager.generate_prekey_bundle(1, Some(1), Some(1))
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to load existing PreKeyBundle — falling through to generate");
            manager.generate_prekey_bundle(1, Some(1), Some(1))
        }
    };
    let bundle_bytes = match bundle_result {
        Ok(bundle) => match serde_json::to_vec(&bundle) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize PreKeyBundle for DHT publication");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "failed to obtain PreKeyBundle — sessions will still work via respond_to_session");
            None
        }
    };

    *state.signal_manager.write() = Some(std::sync::Arc::new(SignalManagerHandle {
        manager: manager.with_session_cache(256),
    }));

    // Store the Ed25519 secret key bytes so message_service can sign envelopes
    *state.identity_secret.lock() = Some(*secret_key);

    bundle_bytes
}
