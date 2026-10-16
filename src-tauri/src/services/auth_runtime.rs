//! The auth commands' Tauri-runtime orchestration: create and unlock (the
//! lifecycle gate, the session scope, background services), logout and
//! delete (the session teardown, then `host::identity`), and the login
//! picker's list (the registry, plan D1).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Manager as _;

use crate::db_helpers::db_call;
use crate::services;
use crate::services::auth_cores::identity_error_message;
use crate::state::{AppState, SharedState};
use crate::state_helpers;
use rekindle_db::paths::DataRoot;
use rekindle_db::Db;
use rekindle_node::host::identity;

/// Summary of a persisted identity, used by the account picker.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummary {
    pub public_key: String,
    pub display_name: String,
    pub created_at: i64,
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

pub async fn logout_inner(app: tauri::AppHandle, state: Arc<AppState>) -> Result<(), String> {
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

    services::session::end_session(services::session::SessionEnd::Logout(&app), &state).await;

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
) -> Result<(), String> {
    use crate::services::auth_cores::{delete_identity_core, verify_identity_passphrase_core};

    let root = app.state::<DataRoot>().inner().clone();
    // The passphrase is checked first (and counted when wrong); only then
    // is the active identity's session ended.
    verify_identity_passphrase_core(&root, &state, &public_key, &passphrase).await?;

    let is_active = state
        .identity
        .read()
        .as_ref()
        .is_some_and(|id| id.public_key == public_key);

    // The active identity is logged out first, the same teardown as the
    // logout command (lifecycle, session, windows, the login window).
    if is_active {
        logout_inner(app.clone(), Arc::clone(&state)).await?;
    }

    delete_identity_core(&root, &state, &public_key, &passphrase).await?;
    tracing::info!(public_key = %public_key, "identity deleted");
    Ok(())
}

pub async fn create_identity_inner(
    passphrase: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: Arc<AppState>,
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::create_identity_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let root = app.state::<DataRoot>().inner().clone();
    let login_scope = begin_unlock(&app, &state).await?;

    let (result, _) =
        match create_identity_core(&root, &passphrase, display_name, &state, Some(&app)).await {
            Ok(v) => v,
            Err(e) => return Err(fail_unlock(&state, e).await),
        };
    let pool = state.db.current()?;

    if let Err(e) = start_background_services(
        &app,
        &state,
        &login_scope,
        &pool,
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
        return Err(abort_unlock(&app, &state, e).await);
    }

    if let Err(closed) =
        crate::services::friendship::spawn_coordinator(&state, app.clone(), &login_scope)
    {
        return Err(abort_unlock(&app, &state, closed.to_string()).await);
    }

    if let Err(e) = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational)
    {
        tracing::error!(error = %e, "post-create-identity: failed to reach Operational — commands will be gated");
    }

    Ok(result)
}

/// Wait for Starting → Locked (async Veilid attach), move to Resuming and
/// begin the login session. Mirrors Briar's waitForStartup() / the daemon's
/// can_unlock() gate: the timeout only bounds the wait; the checked
/// transition validates the actual state and fails loud instead of
/// stranding the FSM in Locked. The session begins before the identity
/// loads: loading it already spawns session work (governance re-merge,
/// reliability flush).
async fn begin_unlock(
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<Arc<rekindle_lifecycle::SessionScope>, String> {
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        state.lifecycle.wait_until_unlockable(),
    )
    .await;
    state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Resuming)
        .map_err(|e| format!("network not ready ({e}) — wait for the node to connect and retry"))?;
    Ok(services::session::begin(app, state))
}

/// Undo a create or unlock that failed before its session started: stop
/// what the attempt spawned, take out anything it installed, and return to
/// `Locked`.
async fn fail_unlock(state: &SharedState, error: String) -> String {
    services::session::stop_scope(state).await;
    services::session::lock_identity(state).await;
    let _ = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Locked);
    error
}

/// Undo an unlock whose post-unlock setup failed: lock the vault, drop
/// every piece of per-user state, return to `Locked`, and hand back `error`.
async fn abort_unlock(app: &tauri::AppHandle, state: &Arc<AppState>, error: String) -> String {
    tracing::error!(error = %error, "post-unlock setup failed — locking again");
    services::session::end_session(services::session::SessionEnd::Logout(app), state).await;
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
) -> Result<crate::services::auth_cores::LoginResult, String> {
    use crate::commands::auth::login_core;
    use crate::services::login_runtime::{start_background_services, DhtKeysConfig};

    let root = app.state::<DataRoot>().inner().clone();
    let login_scope = begin_unlock(&app, &state).await?;

    let (result, _, dht_cols) =
        match login_core(&root, &public_key, &passphrase, &state, Some(&app)).await {
            Ok(v) => v,
            Err(e) => return Err(fail_unlock(&state, e).await),
        };
    let pool = state.db.current()?;

    if let Err(e) = start_background_services(
        &app,
        &state,
        &login_scope,
        &pool,
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
        return Err(abort_unlock(&app, &state, e).await);
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
        return Err(abort_unlock(&app, &state, closed.to_string()).await);
    }

    if let Err(e) = state
        .lifecycle
        .transition(rekindle_lifecycle::LifecycleState::Operational)
    {
        tracing::error!(error = %e, "post-login: failed to reach Operational — commands will be gated");
    }

    Ok(result)
}

/// The identities in the registry, for the login picker (readable while
/// locked: it holds no secret).
///
/// # Errors
/// The registry cannot be read.
pub fn list_identities_inner(root: &DataRoot) -> Result<Vec<IdentitySummary>, String> {
    Ok(identity::list(root)
        .map_err(|e| identity_error_message(&e))?
        .into_iter()
        .map(|entry| IdentitySummary {
            public_key: entry.public_key,
            display_name: entry.display_name,
            created_at: entry.created_at,
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
