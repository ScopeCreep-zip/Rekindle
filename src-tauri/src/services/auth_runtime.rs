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

use crate::db_helpers::db_call;
use crate::keystore::{KeystoreHandle, StrongholdKeystore};
use crate::services;
use crate::state::{AppState, SharedState};
use crate::state_helpers;
use rekindle_db::Db;

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
    // The lifecycle is the single-flight gate: a logout already under way
    // (or no session) refuses the transition, and a second concurrent
    // teardown is never started (plan C4.L2).
    state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Locking)
        .map_err(|e| format!("cannot log out now: {e}"))?;

    let active_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone());

    services::session::end_session(
        services::session::SessionEnd::Logout(&app),
        &state,
        &keystore_handle,
    )
    .await;

    let preselect = active_key
        .as_deref()
        .map(rekindle_types::key_format::public_key_hex)
        .transpose()
        .map_err(|e| format!("active identity key: {e}"))?;
    crate::windows::open_login(&app, preselect.as_ref())?;

    for (label, window) in app.webview_windows() {
        if label != crate::window_labels::LOGIN {
            let _ = window.destroy();
        }
    }

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
    pool: Db,
    keystore_handle: KeystoreHandle,
) -> Result<(), String> {
    let config_dir = app.state::<rekindle_db::paths::DataRoot>().config.clone();

    StrongholdKeystore::initialize_for_identity(&config_dir, &public_key, &passphrase)
        .map_err(|e| crate::keystore::map_stronghold_error(&e))?;

    let is_active = state
        .identity
        .read()
        .as_ref()
        .is_some_and(|id| id.public_key == public_key);

    if is_active {
        services::session::end_session(
            services::session::SessionEnd::Logout(&app),
            &state,
            &keystore_handle,
        )
        .await;
        for (label, window) in app.webview_windows() {
            if label != crate::window_labels::LOGIN {
                let _ = window.destroy();
            }
        }
    }

    let pk = public_key.clone();
    db_call(&pool, move |conn| {
        rekindle_db::repo::identity::delete(conn, &pk)
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
    pool: Db,
    keystore_handle: KeystoreHandle,
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::create_identity_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let config_dir = app.state::<rekindle_db::paths::DataRoot>().config.clone();

    // Wait for Starting → Locked (async Veilid attach) before unlocking —
    // mirrors Briar's waitForStartup() / the daemon's can_unlock() gate. The
    // timeout only bounds the wait; the real gate is the checked transition
    // below, which validates the actual current state and fails loud instead
    // of silently stranding the FSM in Locked.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        state.lifecycle.wait_until_unlockable(),
    )
    .await;
    state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Resuming)
        .map_err(|e| format!("network not ready ({e}) — wait for the node to connect and retry"))?;
    // The session begins before the identity loads: loading it already
    // spawns session work (governance re-merge, reliability flush).
    let login_scope = services::session::begin(&app, &state);

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
            services::session::stop_scope(&state).await;
            let _ = state
                .lifecycle
                .transition(rekindle_lifecycle::LifecycleState::Locked);
            return Err(e);
        }
    };

    if let Err(e) = start_background_services(
        &app,
        &state,
        &login_scope,
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
    )
    .await
    {
        return Err(abort_unlock(&app, &state, &keystore_handle, e).await);
    }

    if let Err(closed) =
        crate::services::friendship::spawn_coordinator(&state, app.clone(), &login_scope)
    {
        return Err(abort_unlock(&app, &state, &keystore_handle, closed.to_string()).await);
    }

    if let Err(e) = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational)
    {
        tracing::error!(error = %e, "post-create-identity: failed to reach Operational — commands will be gated");
    }

    Ok(result)
}

/// Undo an unlock whose post-unlock setup failed: lock the vault, drop
/// every piece of per-user state, return to `Locked`, and hand back `error`.
async fn abort_unlock(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    keystore_handle: &KeystoreHandle,
    error: String,
) -> String {
    tracing::error!(error = %error, "post-unlock setup failed — locking again");
    services::session::end_session(
        services::session::SessionEnd::Logout(app),
        state,
        keystore_handle,
    )
    .await;
    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Locked);
    error
}

pub async fn login_inner(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: Arc<AppState>,
    pool: Db,
    keystore_handle: KeystoreHandle,
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::login_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let config_dir = app.state::<rekindle_db::paths::DataRoot>().config.clone();

    // Wait for Starting → Locked (async Veilid attach) before unlocking —
    // mirrors Briar's waitForStartup() / the daemon's can_unlock() gate. The
    // timeout only bounds the wait; the real gate is the checked transition
    // below, which validates the actual current state and fails loud instead
    // of silently stranding the FSM in Locked.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        state.lifecycle.wait_until_unlockable(),
    )
    .await;
    state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Resuming)
        .map_err(|e| format!("network not ready ({e}) — wait for the node to connect and retry"))?;
    // The session begins before the identity loads: loading it already
    // spawns session work (governance re-merge, reliability flush).
    let login_scope = services::session::begin(&app, &state);

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
            services::session::stop_scope(&state).await;
            let _ = state
                .lifecycle
                .transition(rekindle_lifecycle::LifecycleState::Locked);
            return Err(e);
        }
    };

    if let Err(e) = start_background_services(
        &app,
        &state,
        &login_scope,
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
    )
    .await
    {
        return Err(abort_unlock(&app, &state, &keystore_handle, e).await);
    }

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

    if let Err(closed) =
        crate::services::friendship::spawn_coordinator(&state, app.clone(), &login_scope)
    {
        return Err(abort_unlock(&app, &state, &keystore_handle, closed.to_string()).await);
    }

    if let Err(e) = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational)
    {
        tracing::error!(error = %e, "post-login: failed to reach Operational — commands will be gated");
    }

    Ok(result)
}

pub async fn list_identities_inner(pool: &Db) -> Result<Vec<IdentitySummary>, String> {
    use base64::Engine as _;
    let rows = db_call(pool, |conn| rekindle_db::repo::identity::list(conn)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let avatar_base64 = row
                .avatar_webp
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
            IdentitySummary {
                public_key: row.public_key,
                display_name: row.display_name,
                created_at: row.created_at,
                has_avatar: avatar_base64.is_some(),
                avatar_base64,
            }
        })
        .collect())
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
        .current_bundle()
        .map_err(|e| format!("current bundle: {e}"))?;
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
    pool: &Db,
) -> Result<crate::audit_repo::AuditVerifyResult, String> {
    let owner = state_helpers::current_owner_key(state)?;
    Ok(crate::audit_repo::verify_async(&app, state, pool, &owner).await)
}

pub async fn audit_export_inner(
    state: &SharedState,
    pool: &Db,
    since: u64,
) -> Result<Vec<rekindle_audit::AuditEntry>, String> {
    let owner = state_helpers::current_owner_key(state)?;
    let owner_clone = owner.clone();
    db_call(pool, move |conn| {
        rekindle_db::repo::audit::load_since(conn, &owner_clone, since)
    })
    .await
    .map_err(|e| format!("audit export: {e}"))
}
