//! Phase 23.C — auth `_core` helpers lifted from `commands/auth.rs`.
//! Hosts `create_identity_core`, `login_core`, and the shared
//! `initialize_audit_chain` helper. The `_core` naming pattern is
//! preserved so existing E2E test imports continue to work via the
//! `pub use` re-export from `commands::auth`.

use rekindle_vault::VaultKey;

use serde::{Deserialize, Serialize};

use crate::db;
use crate::db_helpers::db_call;
use crate::keystore::{KeystoreHandle, StrongholdKeystore};
use crate::services;
use crate::state::{IdentityState, SharedState, UserStatus};
use rekindle_db::Db;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub public_key: String,
    pub display_name: String,
}

pub use rekindle_db::repo::identity::IdentityDhtColumns;

pub async fn create_identity_core(
    config_dir: &std::path::Path,
    passphrase: &str,
    display_name: Option<String>,
    state: &SharedState,
    pool: &Db,
    keystore_handle: &KeystoreHandle,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(LoginResult, [u8; 32]), String> {
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();

    std::fs::create_dir_all(config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;

    let identity = rekindle_crypto::Identity::generate();
    let public_key = identity.public_key_hex();
    let secret_bytes = *identity.secret_key_bytes();
    let display_name = display_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("User_{}", rekindle_utils::text::prefix(&public_key, 8)));
    let now = db::timestamp_now();

    let keystore = StrongholdKeystore::initialize_for_identity(config_dir, &public_key, passphrase)
        .map_err(|e| crate::keystore::map_stronghold_error(&e))?;
    keystore
        .vault_put(&VaultKey::IdentityEd25519, &secret_bytes)
        .map_err(|e| e.to_string())?;

    *keystore_handle.lock() = Some(keystore);

    let pk = public_key.clone();
    let dn = display_name.clone();
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::insert(conn, &pk, &dn, now)
    })
    .await?;

    let identity_state = IdentityState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    };

    *state.identity.write() = Some(identity_state);

    initialize_audit_chain(app_handle, state, pool, keystore_handle, &public_key).await;
    if let Some(app) = app_handle {
        let _ = crate::audit_repo::verify_async(app, state, pool, &public_key).await;
    }
    crate::audit_repo::append_async(
        state,
        pool,
        &public_key,
        rekindle_audit::AuditKind::VaultUnlocked,
        serde_json::json!({ "first_time": true }),
    )
    .await;

    let result = LoginResult {
        public_key,
        display_name,
    };
    Ok((result, secret_bytes))
}

async fn initialize_audit_chain(
    app_handle: Option<&tauri::AppHandle>,
    state: &SharedState,
    pool: &Db,
    keystore: &KeystoreHandle,
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
    config_dir: &std::path::Path,
    public_key: &str,
    passphrase: &str,
    state: &SharedState,
    pool: &Db,
    keystore_handle: &KeystoreHandle,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(LoginResult, [u8; 32], IdentityDhtColumns), String> {
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();

    let pk_query = public_key.to_string();
    let (display_name, dht_cols) = db_call(pool, move |conn| {
        rekindle_db::repo::identity::login_row(conn, &pk_query)
    })
    .await?
    .ok_or_else(|| "no identity found — please create one first".to_string())?;

    let keystore = StrongholdKeystore::initialize_for_identity(config_dir, public_key, passphrase)
        .map_err(|e| {
            tracing::warn!(
                public_key = %public_key,
                error = %e,
                "Stronghold unlock failed"
            );
            crate::keystore::map_stronghold_error(&e)
        })?;

    let secret_bytes = keystore
        .vault_get(&VaultKey::IdentityEd25519)
        .map_err(|e| e.to_string())?;

    let secret = secret_bytes.ok_or_else(|| {
        "No private key found in keystore — identity may be corrupted. \
         Please create a new identity."
            .to_string()
    })?;
    let key_array: [u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| "invalid secret key length in Stronghold")?;
    let restored = rekindle_crypto::Identity::from_secret_bytes(&key_array);
    let restored_pub = restored.public_key_hex();
    if restored_pub != public_key {
        return Err("Wrong passphrase — decrypted key does not match stored identity".to_string());
    }

    *keystore_handle.lock() = Some(keystore);

    let identity_state = IdentityState {
        public_key: public_key.to_string(),
        display_name: display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    };
    *state.identity.write() = Some(identity_state);

    crate::community_loader::load_friends_from_db(pool, state, public_key).await?;
    crate::community_loader::load_communities_from_db(pool, state, public_key).await?;
    services::community::hydrate_peer_reliability(state, pool).await;

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
        crate::community_loader::restore_governance_from_cache(pool, state, public_key).await
    {
        tracing::warn!(%error, "failed to restore governance from local cache");
    }

    initialize_audit_chain(app_handle, state, pool, keystore_handle, public_key).await;
    if let Some(app) = app_handle {
        let _ = crate::audit_repo::verify_async(app, state, pool, public_key).await;
    }
    crate::audit_repo::append_async(
        state,
        pool,
        public_key,
        rekindle_audit::AuditKind::VaultUnlocked,
        serde_json::json!({ "first_time": false }),
    )
    .await;

    let result = LoginResult {
        public_key: public_key.to_string(),
        display_name,
    };
    Ok((result, key_array, dht_cols))
}
