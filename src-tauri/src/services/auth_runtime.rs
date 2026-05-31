//! Phase 23.C — auth Tauri-runtime orchestration lifted from
//! `commands/auth.rs`. Hosts `logout_inner` and `delete_identity_inner`
//! — the long teardown sequences (zeroize keystore, signal every
//! background-service shutdown channel, publish Offline, close DHT
//! state, destroy non-login windows) that the Tauri handlers used to
//! inline. Per Invariant 7 these are legitimate Tauri-runtime glue
//! (multi-step orchestration over AppState + AppHandle + KeystoreHandle).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Manager as _;

use crate::commands::voice::{shutdown_voice, VoiceShutdownOpts};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::keystore::{KeystoreHandle, StrongholdKeystore};
use crate::services;
use crate::state::{AppState, SharedState, UserStatus};
use crate::state_helpers;

/// Summary of a persisted identity, used by the account picker.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummary {
    pub public_key: String,
    pub display_name: String,
    pub created_at: i64,
    pub has_avatar: bool,
    pub avatar_base64: Option<String>,
}

/// Phase 3b debug diagnostic — inspect the local PQXDH PreKeyBundle.
///
/// Returns the byte lengths of each bundle component so the dev console
/// can confirm ML-KEM-768 keys (1184 B public, 1088 B ciphertext) and
/// classical X3DH keys (32 B X25519, 64 B Ed25519 sig) are wired through
/// the full publish path. Debug-only — gated on `cfg(debug_assertions)`
/// so release builds never surface key sizes.
#[cfg(debug_assertions)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PqxdhBundleInfo {
    pub identity_key_len: usize,
    pub signed_prekey_len: usize,
    pub signed_prekey_signature_len: usize,
    pub one_time_prekey_len: Option<usize>,
    pub registration_id: u32,
    pub pqpk_lr_len: usize,
    pub pqpk_lr_signature_len: usize,
    pub pqpk_ot_len: Option<usize>,
    pub pqpk_ot_signature_len: Option<usize>,
}

pub async fn logout_inner(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    keystore_handle: KeystoreHandle,
) -> Result<(), String> {
    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Locking);

    keystore_handle.lock().take();

    let sync_tx = state.sync_shutdown_tx.read().clone();
    if let Some(tx) = sync_tx {
        let _ = tx.send(()).await;
    }

    let game_shutdown_tx = state
        .game_detector
        .lock()
        .as_ref()
        .map(|h| h.shutdown_tx.clone());
    if let Some(tx) = game_shutdown_tx {
        let _ = tx.send(()).await;
    }

    shutdown_voice(&state, &VoiceShutdownOpts::FULL).await;

    {
        let tx = state.route_refresh_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    {
        let tx = state.idle_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }
    *state.pre_away_status.write() = None;

    {
        let tx = state.heartbeat_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    if state_helpers::identity_status(&state) != Some(UserStatus::Offline) {
        if let Err(e) =
            services::presence_service::publish_status(&state, UserStatus::Offline).await
        {
            tracing::warn!(error = %e, "failed to publish offline status on logout");
        }
    }

    let active_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone());

    services::veilid::logout_cleanup(Some(&app), &state).await;

    crate::windows::open_login(&app, active_key.as_deref())?;

    for (label, window) in app.webview_windows() {
        if label != "login" {
            let _ = window.destroy();
        }
    }

    *state.sync_shutdown_tx.write() = None;
    *state.game_detector.lock() = None;

    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Locked);

    Ok(())
}

pub async fn delete_identity_inner(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: Arc<AppState>,
    pool: DbPool,
    keystore_handle: KeystoreHandle,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    StrongholdKeystore::initialize_for_identity(&config_dir, &public_key, &passphrase)
        .map_err(|e| crate::keystore::map_stronghold_error(&e))?;

    let is_active = state
        .identity
        .read()
        .as_ref()
        .is_some_and(|id| id.public_key == public_key);

    if is_active {
        keystore_handle.lock().take();

        let sync_tx = state.sync_shutdown_tx.read().clone();
        if let Some(tx) = sync_tx {
            let _ = tx.send(()).await;
        }
        let game_shutdown_tx = state
            .game_detector
            .lock()
            .as_ref()
            .map(|h| h.shutdown_tx.clone());
        if let Some(tx) = game_shutdown_tx {
            let _ = tx.send(()).await;
        }

        shutdown_voice(&state, &VoiceShutdownOpts::FULL).await;

        {
            let tx = state.route_refresh_shutdown_tx.write().take();
            if let Some(tx) = tx {
                let _ = tx.send(()).await;
            }
        }

        {
            let tx = state.idle_shutdown_tx.write().take();
            if let Some(tx) = tx {
                let _ = tx.send(()).await;
            }
        }
        *state.pre_away_status.write() = None;

        if state_helpers::identity_status(&state) != Some(UserStatus::Offline) {
            let _ = services::presence_service::publish_status(&state, UserStatus::Offline).await;
        }

        services::veilid::logout_cleanup(Some(&app), &state).await;

        for (label, window) in app.webview_windows() {
            if label != "login" {
                let _ = window.destroy();
            }
        }

        *state.sync_shutdown_tx.write() = None;
        *state.game_detector.lock() = None;
    }

    let pk = public_key.clone();
    db_call(&pool, move |conn| {
        conn.execute(
            "DELETE FROM identity WHERE public_key = ?1",
            rusqlite::params![pk],
        )?;
        Ok(())
    })
    .await?;

    StrongholdKeystore::delete_snapshot(&config_dir, &public_key)
        .map_err(|e| format!("failed to delete keystore: {e}"))?;

    tracing::info!(public_key = %public_key, "identity deleted");
    Ok(())
}

pub async fn create_identity_inner(
    passphrase: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: Arc<AppState>,
    pool: DbPool,
    keystore_handle: KeystoreHandle,
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::create_identity_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Resuming);

    let (result, secret_bytes) = match create_identity_core(
        &config_dir,
        &passphrase,
        display_name,
        &state,
        &pool,
        &keystore_handle,
        Some(&app),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = state
                .lifecycle
                .transition(rekindle_lifecycle::LifecycleState::Locked);
            return Err(e);
        }
    };

    start_background_services(
        &app,
        &state,
        &pool,
        &secret_bytes,
        DhtKeysConfig {
            existing_dht_key: None,
            existing_friend_list_key: None,
            dht_owner_keypair: None,
            friend_list_owner_keypair: None,
            account_dht_key: None,
            account_owner_keypair: None,
            mailbox_dht_key: None,
        },
    );

    let coord_handle = crate::services::friendship::spawn_coordinator(&state, app.clone());
    state.background_handles.lock().push(coord_handle);

    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational);

    Ok(result)
}

pub async fn login_inner(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: Arc<AppState>,
    pool: DbPool,
    keystore_handle: KeystoreHandle,
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::login_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Resuming);

    let (result, secret_key, dht_cols) = match login_core(
        &config_dir,
        &public_key,
        &passphrase,
        &state,
        &pool,
        &keystore_handle,
        Some(&app),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = state
                .lifecycle
                .transition(rekindle_lifecycle::LifecycleState::Locked);
            return Err(e);
        }
    };

    start_background_services(
        &app,
        &state,
        &pool,
        &secret_key,
        DhtKeysConfig {
            existing_dht_key: dht_cols.existing_dht_key,
            existing_friend_list_key: dht_cols.existing_friend_list_key,
            dht_owner_keypair: dht_cols.dht_owner_keypair,
            friend_list_owner_keypair: dht_cols.friend_list_owner_keypair,
            account_dht_key: dht_cols.account_dht_key,
            account_owner_keypair: dht_cols.account_owner_keypair,
            mailbox_dht_key: dht_cols.mailbox_dht_key,
        },
    );

    {
        let mut rx = state.network_ready_rx.clone();
        let ready = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if *rx.borrow_and_update() {
                    return true;
                }
                if rx.changed().await.is_err() {
                    return false;
                }
            }
        })
        .await
        .unwrap_or(false);

        if ready {
            if let Err(e) = services::sync_service::sync_friends_now(&state, &app).await {
                tracing::warn!(error = %e, "login-time friend sync failed");
            }
        } else {
            tracing::warn!("network not ready within 20s — buddy list will use fallback sync");
        }
    }

    crate::deep_links::emit_pending_deep_link(&app);

    let coord_handle = crate::services::friendship::spawn_coordinator(&state, app.clone());
    state.background_handles.lock().push(coord_handle);

    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational);

    Ok(result)
}

pub async fn list_identities_inner(pool: &DbPool) -> Result<Vec<IdentitySummary>, String> {
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT public_key, display_name, created_at, avatar_webp \
                 FROM identity ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let avatar_base64 = row
                    .get::<_, Option<Vec<u8>>>("avatar_webp")
                    .unwrap_or(None)
                    .map(|bytes| {
                        use base64::Engine as _;
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    });
                Ok(IdentitySummary {
                    public_key: crate::db::get_str(row, "public_key"),
                    display_name: row.get::<_, String>("display_name").unwrap_or_default(),
                    created_at: crate::db::get_i64(row, "created_at"),
                    has_avatar: avatar_base64.is_some(),
                    avatar_base64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

#[cfg(debug_assertions)]
pub fn pqxdh_bundle_info_inner(state: &Arc<AppState>) -> Result<PqxdhBundleInfo, String> {
    let handle = state
        .signal_manager
        .read()
        .as_ref()
        .map(std::sync::Arc::clone)
        .ok_or("signal manager not initialized")?;
    let bundle = handle
        .manager
        .load_existing_prekey_bundle(1, Some(1), Some(1))
        .map_err(|e| format!("load existing bundle: {e}"))?
        .ok_or("no bundle in store — log in first")?;
    Ok(PqxdhBundleInfo {
        identity_key_len: bundle.identity_key.len(),
        signed_prekey_len: bundle.signed_prekey.len(),
        signed_prekey_signature_len: bundle.signed_prekey_signature.len(),
        one_time_prekey_len: bundle.one_time_prekey.as_ref().map(Vec::len),
        registration_id: bundle.registration_id,
        pqpk_lr_len: bundle.pqpk_lr.len(),
        pqpk_lr_signature_len: bundle.pqpk_lr_signature.len(),
        pqpk_ot_len: bundle.pqpk_ot.as_ref().map(Vec::len),
        pqpk_ot_signature_len: bundle.pqpk_ot_signature.as_ref().map(Vec::len),
    })
}

pub async fn audit_verify_inner(
    app: tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
) -> Result<crate::audit_repo::AuditVerifyResult, String> {
    let owner = state_helpers::current_owner_key(state)?;
    Ok(crate::audit_repo::verify_async(&app, state, pool, &owner).await)
}

pub async fn audit_export_inner(
    state: &SharedState,
    pool: &DbPool,
    since: u64,
) -> Result<Vec<rekindle_audit::AuditEntry>, String> {
    let owner = state_helpers::current_owner_key(state)?;
    let owner_clone = owner.clone();
    db_call(pool, move |conn| {
        crate::audit_repo::load_since(conn, &owner_clone, since)
    })
    .await
    .map_err(|e| format!("audit export: {e}"))
}
