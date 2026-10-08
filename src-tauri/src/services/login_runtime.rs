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

use crate::services;
use crate::state::{SharedState, SignalManagerHandle};
use rekindle_db::Db;

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
pub async fn start_background_services(
    app: &tauri::AppHandle,
    state: &SharedState,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
    pool: &Db,
    secret_key: &[u8; 32],
    dht_keys: DhtKeysConfig,
) -> Result<(), String> {
    // Initialize Signal Protocol session manager (returns serialized PreKeyBundle)
    let prekey_bundle_bytes = initialize_signal_manager(app, state, secret_key)?;

    // The session's record pool, before anything opens a record (C7.3), with
    // the writes the last logout left unsent (C7.6h) held before anything
    // writes.
    match services::record_pool::start(state) {
        Ok(records) => services::record_pool::restore_unsent(state, &records).await,
        Err(e) => tracing::error!(error = %e, "record pool not started"),
    }

    // Start game detection (only after login — avoids burning CPU before auth)
    services::game_service::initialize(state);
    let game_app = app.clone();
    let game_state = Arc::clone(state);
    scope
        .spawn_with_token("game detection", |stop| {
            services::game_service::start_game_detection(game_app, game_state, stop)
        })
        .map_err(|e| e.to_string())?;

    // The Veilid node is already running (started at app startup).
    // Just spawn sync + DHT publish as background tasks.
    super::login_spawn::spawn_login_services(
        app,
        state,
        scope,
        pool.clone(),
        prekey_bundle_bytes,
        dht_keys,
    )
    .map_err(|e| e.to_string())
}

/// Wait (bounded) for network readiness via the network-ready watch
/// channel. Returns `true` once ready, `false` on timeout or channel
/// close. Readiness is `public_internet_ready` AND at least one live peer
/// in the routing table (0.5.7's richer attachment signal) — DHT
/// record/route opens issued before that are unreliable on a
/// freshly-attached node (sparse routing table → transient `KeyNotFound`),
/// so callers gate their startup DHT work on this.
/// (`AppState.network_ready_rx` is fed by the attachment handler in
/// `services::veilid::network`.)
pub(super) async fn wait_for_network_ready(
    mut rx: tokio::sync::watch::Receiver<bool>,
    timeout_secs: u64,
) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
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
    .unwrap_or(false)
}

/// Publish this identity's records once the network is ready: mailbox,
/// profile, friend list, account (our routes are `OwnRoutes`', plan C7.9b). Each step stops before the next
/// once `stop` is cancelled (plan C4.L1). The record publishes are
/// record-pool writes: at logout the pool's drain releases a step waiting
/// on one (plan C7.6g).
pub(super) async fn spawn_dht_publish(
    app_handle: tauri::AppHandle,
    state: SharedState,
    pool: Db,
    prekey_bundle_bytes: Vec<u8>,
    dht_keys: DhtKeysConfig,
    stop: tokio_util::sync::CancellationToken,
) {
    // Wait for public internet ready before publishing (route/record opens are
    // unreliable on a sparse cold-start routing table).
    let Some(ready) = stop
        .run_until_cancelled(wait_for_network_ready(state.network_ready_rx.clone(), 60))
        .await
    else {
        return;
    };

    if !ready {
        tracing::warn!(
            "timed out waiting for public internet readiness (60s) — \
             DHT publish deferred to sync loop"
        );
        return;
    }

    // Our general route was allocated when the node turned ready, before
    // login (plan C7.9b), so it is usually here already; when it is not, the
    // route republisher publishes it when it lands.
    let route_blob = crate::state_helpers::our_route_blob(&state);
    if route_blob.is_some() {
        recover_behind_meks(&app_handle, &state);
    }

    // Create or open mailbox DHT record
    if stop.is_cancelled() {
        return;
    }
    if let Err(e) = services::dht_publish_service::publish_mailbox(
        &state,
        &pool,
        dht_keys.mailbox_dht_key.as_ref(),
        route_blob.as_deref(),
    )
    .await
    {
        publish_step_failed(&stop, "mailbox", &e);
    }

    if stop.is_cancelled() {
        return;
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
        publish_step_failed(&stop, "profile", &e);
    }

    if stop.is_cancelled() {
        return;
    }
    if let Err(e) = services::dht_publish_service::publish_friend_list(
        &state,
        &pool,
        dht_keys.existing_friend_list_key,
        dht_keys.friend_list_owner_keypair,
    )
    .await
    {
        publish_step_failed(&stop, "friend list", &e);
    }

    if stop.is_cancelled() {
        return;
    }
    // Immediate friend sync now that network is up
    if let Err(e) = services::sync_service::sync_friends_now(&state, &app_handle).await {
        publish_step_failed(&stop, "friend sync", &e);
    }

    if stop.is_cancelled() {
        return;
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
        publish_step_failed(&stop, "account", &e);
    }
}

/// Architecture §7.3 login catch-up: re-acquire any community MEK that is
/// ABSENT or BEHIND the governance generation, rather than waiting for
/// incidental incoming traffic to trigger acquisition. A no-op when every
/// key is current, so it runs whenever our general route is (re)published:
/// the request's replies need a route to reach us. `c.mek_generation` is the
/// authoritative target (restored from SQLite at login, never clobbered).
///
/// Every member takes the same path: `spawn_community_mek_recovery`
/// requests first regardless and mints only if the deterministic election
/// picks this node (under `o_cnt: 0` a creator-only recovery could never
/// recover a community whose creator never returns).
pub(crate) fn recover_behind_meks(app_handle: &tauri::AppHandle, state: &SharedState) {
    let behind: Vec<(String, String)> = {
        let communities = state.communities.read();
        communities
            .values()
            .filter_map(|c| {
                let cached_gen = crate::state_helpers::current_mek(
                    state,
                    &c.id,
                    rekindle_types::channel_keys::KeyScope::Community,
                )
                .map(|mek| mek.generation());
                let is_behind = cached_gen.is_none_or(|g| g < c.mek_generation);
                if !is_behind {
                    return None;
                }
                c.my_pseudonym_key.clone().map(|p| (c.id.clone(), p))
            })
            .collect()
    };
    for (cid, pseudonym) in behind {
        services::community::mek_rotation::spawn_community_mek_recovery(
            app_handle.clone(),
            state.clone(),
            cid,
            pseudonym,
        );
    }
}

/// Report a failed publish step. A step the session's end cut short (the
/// record pool's drain refused or released its call) is not a failure.
fn publish_step_failed(
    stop: &tokio_util::sync::CancellationToken,
    step: &'static str,
    error: &str,
) {
    if stop.is_cancelled() {
        tracing::debug!(step, error, "DHT publish step ended with the session");
    } else {
        tracing::warn!(
            step,
            error,
            "DHT publish step failed — will retry on next sync"
        );
    }
}

/// Initialize the Signal Protocol session manager with the identity key.
///
/// Builds the vault-backed identity, prekey and session stores, then
/// returns the serialized current `PreKeyBundle` for DHT profile subkey 5.
/// Any failure aborts login: without its prekeys the account cannot
/// establish or answer a session.
fn initialize_signal_manager(
    app: &tauri::AppHandle,
    state: &SharedState,
    secret_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
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
        crate::signal_stores::StrongholdPreKeyStore::new(keystore_handle.inner().clone())
            .map_err(|e| format!("load Signal prekeys: {e}"))?;
    let session_store =
        crate::signal_stores::StrongholdSessionStore::new(keystore_handle.inner().clone())
            .map_err(|e| format!("load Signal sessions: {e}"))?;

    let manager = SignalSessionManager::new(
        Box::new(identity_store),
        Box::new(prekey_store),
        Box::new(session_store),
    );

    // Mints the signed prekey and PQ last-resort key on first login only;
    // afterwards the same keys come back, so peers' cached bundles stay valid.
    let bundle = manager
        .current_bundle()
        .map_err(|e| format!("Signal prekey bundle: {e}"))?;
    let bundle_bytes =
        serde_json::to_vec(&bundle).map_err(|e| format!("serialize PreKeyBundle: {e}"))?;

    *state.signal_manager.write() = Some(std::sync::Arc::new(SignalManagerHandle {
        manager: manager.with_session_cache(256),
    }));

    // Store the Ed25519 secret key bytes so message_service can sign envelopes
    *state.identity_secret.lock() = Some(*secret_key);

    Ok(bundle_bytes)
}
