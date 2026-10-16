//! The desktop's identity create and unlock (plan D1): both call
//! `rekindle_node::host::identity`, the one implementation every host
//! uses, and then load the session's in-memory state from the database
//! the unlock opened. `_core` functions take no `AppHandle` requirement so
//! the E2E server and tests drive them directly.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::services;
pub use crate::services::login_runtime::DhtKeysConfig;
use crate::state::{AppState, IdentityState, SharedState, SignalManagerHandle, UserStatus};
use crate::state_helpers;
use rekindle_db::paths::DataRoot;
use rekindle_db::Db;
use rekindle_node::host::identity::{self, IdentityError, Stores, Unlocked};
use rekindle_vault::typed::owner_keys::OwnerKey;
use rekindle_vault::VaultHandle;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub public_key: String,
    pub display_name: String,
}

/// The host's identity slots in `state`.
pub fn stores(state: &AppState) -> Stores<'_> {
    Stores {
        db: &state.db,
        vault: &state.keystore,
        attempts: &state.identity_attempts,
    }
}

/// The message the login window shows for an identity error.
#[must_use]
pub fn identity_error_message(e: &IdentityError) -> String {
    match e {
        IdentityError::NotFound => "no identity found — please create one first".into(),
        IdentityError::WrongPassphrase => "Wrong passphrase".into(),
        IdentityError::Locked { retry_after_ms } => format!(
            "Too many wrong passphrases. Try again in {} s.",
            retry_after_ms.div_ceil(1000)
        ),
        other => other.to_string(),
    }
}

/// Clear the previous session's in-memory state.
fn clear_session_state(state: &SharedState) {
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();
}

/// Hand the unlock's keys and friend authority to the session and return
/// the database the unlock installed.
fn adopt(state: &SharedState, unlocked: Unlocked) -> Result<(Db, LoginResult, [u8; 32]), String> {
    let Unlocked {
        public_key,
        display_name,
        secret,
        signal,
        friend_store,
    } = unlocked;
    *state.friend_store.write() = Some(friend_store);
    *state.signal_manager.write() = Some(Arc::new(SignalManagerHandle { manager: signal }));
    *state.identity_secret.lock() = Some(*secret);
    *state.identity.write() = Some(IdentityState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    });
    let pool = state.db.current()?;
    Ok((
        pool,
        LoginResult {
            public_key,
            display_name,
        },
        *secret,
    ))
}

pub async fn create_identity_core(
    root: &DataRoot,
    passphrase: &str,
    display_name: Option<String>,
    state: &SharedState,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(LoginResult, [u8; 32]), String> {
    clear_session_state(state);
    let unlocked = identity::create(
        root,
        &stores(state),
        display_name.as_deref(),
        passphrase,
        rekindle_utils::timestamp_ms_i64(),
    )
    .await
    .map_err(|e| identity_error_message(&e))?;
    let (pool, result, secret_bytes) = adopt(state, unlocked)?;
    services::preferences::load(state, &pool, &result.public_key).await?;

    initialize_audit_chain(
        app_handle,
        state,
        &pool,
        &state.keystore,
        &result.public_key,
    )
    .await;
    if let Some(app) = app_handle {
        let _ = crate::audit_repo::verify_async(app, state, &pool, &result.public_key).await;
    }
    crate::audit_repo::append_async(
        state,
        &pool,
        &result.public_key,
        rekindle_audit::AuditKind::VaultUnlocked,
        serde_json::json!({ "first_time": true }),
    )
    .await;

    Ok((result, secret_bytes))
}

async fn initialize_audit_chain(
    app_handle: Option<&tauri::AppHandle>,
    state: &SharedState,
    pool: &Db,
    keystore: &VaultHandle,
    owner_key: &str,
) {
    let mac_key = {
        let ks = keystore.lock();
        let Some(ks_ref) = ks.as_ref() else {
            tracing::warn!("audit chain init skipped — keystore not initialized");
            return;
        };
        match crate::keystore::load_or_create_audit_mac_key(ks_ref) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(error = %e, "audit MAC key load failed — chain stays disabled");
                return;
            }
        }
    };
    if let Err(e) =
        crate::audit_repo::restore_chain(app_handle, state, pool, owner_key, mac_key).await
    {
        tracing::warn!(error = %e, owner = %owner_key, "audit chain restore failed");
    }
}

pub async fn login_core(
    root: &DataRoot,
    public_key: &str,
    passphrase: &str,
    state: &SharedState,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(LoginResult, [u8; 32], DhtKeysConfig), String> {
    clear_session_state(state);
    let unlocked = identity::unlock(
        root,
        &stores(state),
        public_key,
        passphrase,
        rekindle_utils::timestamp_ms(),
    )
    .await
    .map_err(|e| {
        tracing::warn!(public_key = %public_key, error = %e, "identity unlock failed");
        identity_error_message(&e)
    })?;
    let (pool, result, key_array) = adopt(state, unlocked)?;
    services::preferences::load(state, &pool, public_key).await?;
    let keystore_handle = &state.keystore;

    let dht_keys = owned_records(state, &pool, public_key).await?;

    crate::community_loader::load_friends_from_db(&pool, state, public_key).await?;
    crate::community_loader::load_communities_from_db(&pool, state, public_key).await?;
    services::community::hydrate_peer_reliability(state, &pool).await;

    crate::community_loader::restore_community_pseudonyms_and_meks(
        state,
        keystore_handle,
        &key_array,
    );

    // Re-merge last-known governance from the warm local cache so
    // communities are usable immediately on open. Must follow the
    // pseudonym restore above (set_governance_state reads
    // my_pseudonym_key). The background DHT rebuild later overwrites
    // this with anything newer.
    if let Err(error) =
        crate::community_loader::restore_governance_from_cache(&pool, state, public_key).await
    {
        tracing::warn!(%error, "failed to restore governance from local cache");
    }

    initialize_audit_chain(app_handle, state, &pool, keystore_handle, public_key).await;
    if let Some(app) = app_handle {
        let _ = crate::audit_repo::verify_async(app, state, &pool, public_key).await;
    }
    crate::audit_repo::append_async(
        state,
        &pool,
        public_key,
        rekindle_audit::AuditKind::VaultUnlocked,
        serde_json::json!({ "first_time": false }),
    )
    .await;

    Ok((result, key_array, dht_keys))
}

/// The records the identity owns: their keys from the identity row, their
/// owner keypairs from the vault.
async fn owned_records(
    state: &SharedState,
    pool: &Db,
    public_key: &str,
) -> Result<DhtKeysConfig, String> {
    let pk = public_key.to_string();
    let cols = crate::db_helpers::db_call(pool, move |conn| {
        rekindle_db::repo::identity::login_row(conn, &pk)
    })
    .await?
    .map(|(_, cols)| cols)
    .ok_or_else(|| "the identity database has no identity row".to_string())?;
    Ok(DhtKeysConfig {
        existing_dht_key: cols.profile,
        existing_friend_list_key: cols.friend_list,
        dht_owner_keypair: state_helpers::load_owner_keypair(state, OwnerKey::Profile)?,
        friend_list_owner_keypair: state_helpers::load_owner_keypair(state, OwnerKey::FriendList)?,
        account_dht_key: cols.account,
        account_owner_keypair: state_helpers::load_owner_keypair(state, OwnerKey::Account)?,
        mailbox_dht_key: cols.mailbox,
    })
}

/// Check `passphrase` against `public_key`'s vault on a blocking thread
/// (Argon2id). A wrong one counts against the identity.
///
/// # Errors
/// Not found, wrong or locked out.
pub async fn verify_identity_passphrase_core(
    root: &DataRoot,
    state: &SharedState,
    public_key: &str,
    passphrase: &str,
) -> Result<(), String> {
    let (root, state, pk, pass) = (
        root.clone(),
        Arc::clone(state),
        public_key.to_string(),
        passphrase.to_string(),
    );
    tokio::task::spawn_blocking(move || {
        identity::verify_passphrase(
            &root,
            &state.identity_attempts,
            &pk,
            &pass,
            rekindle_utils::timestamp_ms(),
        )
    })
    .await
    .map_err(|e| format!("passphrase check: {e}"))?
    .map_err(|e| identity_error_message(&e))
}

/// Delete `public_key`'s vault, database and registry entry (the passphrase
/// is checked again by `host::identity::delete`). The caller has ended the
/// identity's session if it was active.
///
/// # Errors
/// Not found, wrong or locked out, or a file that cannot be removed.
pub async fn delete_identity_core(
    root: &DataRoot,
    state: &SharedState,
    public_key: &str,
    passphrase: &str,
) -> Result<(), String> {
    let (root, state, pk, pass) = (
        root.clone(),
        Arc::clone(state),
        public_key.to_string(),
        passphrase.to_string(),
    );
    tokio::task::spawn_blocking(move || {
        identity::delete(
            &root,
            &state.identity_attempts,
            &pk,
            &pass,
            rekindle_utils::timestamp_ms(),
        )
    })
    .await
    .map_err(|e| format!("identity delete: {e}"))?
    .map_err(|e| identity_error_message(&e))
}
