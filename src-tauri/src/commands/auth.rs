use std::sync::Arc;

use rekindle_crypto::keychain::{KEY_ED25519_PRIVATE, VAULT_IDENTITY};
use rekindle_crypto::Keychain as _;
use rusqlite::OptionalExtension as _;
use serde::{Deserialize, Serialize};
use tauri::{Manager as _, State};
use tokio::sync::mpsc;

use crate::db::{self, DbPool};
use crate::keystore::{KeystoreHandle, StrongholdKeystore};
use crate::services;
use crate::state::{
    ChannelInfo, ChannelType, CommunityState, FriendState, IdentityState,
    SharedState, SignalManagerHandle, UserStatus,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub public_key: String,
    pub display_name: String,
}

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

/// Get the current identity's public key from `AppState`.
///
/// Used by commands that need to scope SQL queries to the active identity.
pub fn current_owner_key(state: &SharedState) -> Result<String, String> {
    state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .ok_or_else(|| "not logged in".to_string())
}

/// Core identity creation logic, separated from `AppHandle` for testability.
///
/// Generates keypair, stores in Stronghold + `SQLite`, sets `AppState`.
/// Returns `(LoginResult, secret_key_bytes)` so the caller can decide
/// whether to spawn background services.
///
/// Multiple identities can coexist — each gets its own Stronghold file
/// and `owner_key`-scoped rows. Only one is active at a time.
pub async fn create_identity_core(
    config_dir: &std::path::Path,
    passphrase: &str,
    display_name: Option<String>,
    state: &SharedState,
    pool: &DbPool,
    keystore_handle: &KeystoreHandle,
) -> Result<(LoginResult, [u8; 32]), String> {
    // Clear in-memory state from any previous session
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();

    // Ensure config directory exists for Stronghold snapshot
    std::fs::create_dir_all(config_dir)
        .map_err(|e| format!("failed to create config dir: {e}"))?;

    let identity = rekindle_crypto::Identity::generate();
    let public_key = identity.public_key_hex();
    let secret_bytes = *identity.secret_key_bytes();
    let display_name = display_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("User_{}", &public_key[..8]));
    let now = db::timestamp_now();

    // Initialize per-identity Stronghold and store the private key
    let keystore =
        StrongholdKeystore::initialize_for_identity(config_dir, &public_key, passphrase)
            .map_err(|e| e.to_string())?;
    keystore
        .store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &secret_bytes)
        .map_err(|e| e.to_string())?;
    keystore.save().map_err(|e| e.to_string())?;

    // Keep the keystore unlocked for the session
    *keystore_handle.lock() = Some(keystore);

    // Persist identity to SQLite (alongside any existing identities)
    let db = pool.clone();
    let pk = public_key.clone();
    let dn = display_name.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO identity (public_key, display_name, created_at) VALUES (?, ?, ?)",
            rusqlite::params![pk, dn, now],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Store the identity in AppState
    let identity_state = IdentityState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    };

    *state.identity.write() = Some(identity_state);

    let result = LoginResult {
        public_key,
        display_name,
    };
    Ok((result, secret_bytes))
}

/// Create a new identity (first-time setup).
///
/// Generates an Ed25519 keypair, stores the private key in Stronghold
/// (encrypted with passphrase via `Argon2id`), and persists public identity to `SQLite`.
#[tauri::command]
pub async fn create_identity(
    passphrase: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<LoginResult, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?;

    let (result, secret_bytes) = create_identity_core(
        &config_dir,
        &passphrase,
        display_name,
        state.inner(),
        pool.inner(),
        keystore_handle.inner(),
    )
    .await?;

    // Spawn background services (non-blocking — returns immediately)
    // New identity has no existing DHT keys or owner keypairs.
    start_background_services(&app, state.inner(), pool.inner(), &secret_bytes, DhtKeysConfig {
        existing_dht_key: None,
        existing_friend_list_key: None,
        dht_owner_keypair: None,
        friend_list_owner_keypair: None,
        account_dht_key: None,
        account_owner_keypair: None,
        mailbox_dht_key: None,
    });

    Ok(result)
}

/// Core login logic, separated from `AppHandle` for testability.
///
/// Loads identity from `SQLite` by `public_key`, unlocks its per-identity
/// Stronghold, verifies keypair, restores friends + communities.
/// Returns `(LoginResult, secret_key, dht_keys)`.
/// Columns loaded from the identity table during login.
#[derive(Debug)]
pub struct IdentityDhtColumns {
    pub existing_dht_key: Option<String>,
    pub existing_friend_list_key: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub friend_list_owner_keypair: Option<String>,
    pub account_dht_key: Option<String>,
    pub account_owner_keypair: Option<String>,
    pub mailbox_dht_key: Option<String>,
}

pub async fn login_core(
    config_dir: &std::path::Path,
    public_key: &str,
    passphrase: &str,
    state: &SharedState,
    pool: &DbPool,
    keystore_handle: &KeystoreHandle,
) -> Result<(LoginResult, [u8; 32], IdentityDhtColumns), String> {
    // Clear in-memory state from any previous session
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();

    // Load identity metadata from SQLite by public key
    let db = pool.clone();
    let pk_query = public_key.to_string();
    let (display_name, dht_cols) =
        tokio::task::spawn_blocking(move || {
            let conn = db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn
                .prepare(
                    "SELECT display_name, dht_record_key, friend_list_dht_key, \
                     dht_owner_keypair, friend_list_owner_keypair, \
                     account_dht_key, account_owner_keypair, mailbox_dht_key \
                     FROM identity WHERE public_key = ?1",
                )
                .map_err(|e| e.to_string())?;
            let row = stmt
                .query_row(rusqlite::params![pk_query], |row| {
                    Ok((
                        row.get::<_, String>("display_name").unwrap_or_default(),
                        IdentityDhtColumns {
                            existing_dht_key: row.get::<_, Option<String>>("dht_record_key")?,
                            existing_friend_list_key: row.get::<_, Option<String>>("friend_list_dht_key")?,
                            dht_owner_keypair: row.get::<_, Option<String>>("dht_owner_keypair")?,
                            friend_list_owner_keypair: row.get::<_, Option<String>>("friend_list_owner_keypair")?,
                            account_dht_key: row.get::<_, Option<String>>("account_dht_key")?,
                            account_owner_keypair: row.get::<_, Option<String>>("account_owner_keypair")?,
                            mailbox_dht_key: row.get::<_, Option<String>>("mailbox_dht_key")?,
                        },
                    ))
                })
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "no identity found — please create one first".to_string())?;
            Ok::<(String, IdentityDhtColumns), String>(row)
        })
        .await
        .map_err(|e| e.to_string())??;

    // Unlock per-identity Stronghold with passphrase and load private key
    let keystore =
        StrongholdKeystore::initialize_for_identity(config_dir, public_key, passphrase)
            .map_err(|e| {
                let msg = e.to_string();
                tracing::warn!(
                    public_key = %public_key,
                    error = %msg,
                    "Stronghold unlock failed"
                );
                if msg.contains("snapshot") || msg.contains("decrypt") {
                    "Wrong passphrase — unable to unlock keystore".to_string()
                } else {
                    msg
                }
            })?;

    let secret_bytes = keystore
        .load_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE)
        .map_err(|e| e.to_string())?;

    // Verify the private key exists and matches the stored public key
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
        return Err(
            "Wrong passphrase — decrypted key does not match stored identity".to_string(),
        );
    }

    // Keep the keystore unlocked for the session
    *keystore_handle.lock() = Some(keystore);

    let identity_state = IdentityState {
        public_key: public_key.to_string(),
        display_name: display_name.clone(),
        status: UserStatus::Online,
        status_message: String::new(),
    };
    *state.identity.write() = Some(identity_state);

    // Restore friends and communities from SQLite into AppState (scoped to this identity)
    load_friends_from_db(pool, state, public_key).await?;
    load_communities_from_db(pool, state, public_key).await?;

    // Derive pseudonyms for each community and load MEKs from Stronghold
    restore_community_pseudonyms_and_meks(state, keystore_handle, &key_array);

    let result = LoginResult {
        public_key: public_key.to_string(),
        display_name,
    };
    Ok((result, key_array, dht_cols))
}

/// Unlock existing identity with passphrase.
///
/// Opens the per-identity Stronghold snapshot with the passphrase, loads the
/// Ed25519 private key, verifies it matches the `SQLite` public key, and
/// restores friends + communities.
#[tauri::command]
pub async fn login(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<LoginResult, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?;

    let (result, secret_key, dht_cols) = login_core(
        &config_dir,
        &public_key,
        &passphrase,
        state.inner(),
        pool.inner(),
        keystore_handle.inner(),
    )
    .await?;

    // Spawn background services (non-blocking — returns immediately)
    start_background_services(
        &app,
        state.inner(),
        pool.inner(),
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

    Ok(result)
}

/// Get the current identity state.
///
/// Used by newly opened windows to hydrate their local auth state
/// from the shared Rust backend (each webview has isolated JS context).
#[tauri::command]
pub async fn get_identity(
    state: State<'_, SharedState>,
) -> Result<Option<LoginResult>, String> {
    let identity = state.identity.read();
    Ok(identity.as_ref().map(|id| LoginResult {
        public_key: id.public_key.clone(),
        display_name: id.display_name.clone(),
    }))
}

/// Log out: save and lock Stronghold, clean up user state, keep node alive.
#[tauri::command]
pub async fn logout(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    // Drop the Stronghold keystore handle — no save needed since private keys
    // are never modified after create_identity. Saving here would trigger an
    // atomic temp-file write that can leave orphaned files on interrupted shutdown.
    keystore_handle.lock().take();

    // Shut down sync service
    let sync_tx = state.sync_shutdown_tx.read().clone();
    if let Some(tx) = sync_tx {
        let _ = tx.send(()).await;
    }

    // Shut down game detection
    let game_shutdown_tx = state
        .game_detector
        .lock()
        .as_ref()
        .map(|h| h.shutdown_tx.clone());
    if let Some(tx) = game_shutdown_tx {
        let _ = tx.send(()).await;
    }

    // Shut down voice engine
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
        *ve = None;
    }

    // Grab the active identity's key before cleanup clears it — the login window
    // will pre-select this account so the user just has to re-enter their
    // passphrase instead of navigating through the picker again.
    let active_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone());

    // Clean up user-specific DHT state (close records, release route, clear identity)
    // but keep the Veilid node alive for re-login
    services::veilid_service::logout_cleanup(Some(&app), &state).await;

    // Re-open the login window (it was closed during show_buddy_list)
    crate::windows::open_login(&app, active_key.as_deref())?;

    // Destroy all windows except login (orphaned windows would have stale state).
    // Using destroy() instead of close() so labels are immediately unregistered
    // and the next login can create fresh windows without label collisions.
    for (label, window) in app.webview_windows() {
        if label != "login" {
            let _ = window.destroy();
        }
    }

    // Clear remaining per-session state not handled by logout_cleanup
    *state.sync_shutdown_tx.write() = None;
    *state.game_detector.lock() = None;

    Ok(())
}

/// List all persisted identities (for the account picker).
///
/// Returns summaries of every identity in `SQLite`, ordered by creation date.
/// No authentication needed — this is called by the login window on mount.
#[tauri::command]
pub async fn list_identities(
    pool: State<'_, DbPool>,
) -> Result<Vec<IdentitySummary>, String> {
    let db = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT public_key, display_name, created_at, avatar_webp \
                 FROM identity ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
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
                    public_key: db::get_str(row, "public_key"),
                    display_name: row.get::<_, String>("display_name").unwrap_or_default(),
                    created_at: db::get_i64(row, "created_at"),
                    has_avatar: avatar_base64.is_some(),
                    avatar_base64,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok::<Vec<IdentitySummary>, String>(rows)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete a specific identity after verifying the passphrase.
///
/// Opens the identity's Stronghold to verify the passphrase, then deletes:
/// - The identity row (CASCADE deletes all scoped data)
/// - The Stronghold snapshot file
///
/// If deleting the currently active identity, performs logout first.
#[tauri::command]
pub async fn delete_identity(
    public_key: String,
    passphrase: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?;

    // Verify passphrase by attempting to open the identity's Stronghold
    StrongholdKeystore::initialize_for_identity(&config_dir, &public_key, &passphrase)
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("snapshot") || msg.contains("decrypt") {
                "Wrong passphrase".to_string()
            } else {
                msg
            }
        })?;

    // If deleting the active identity, logout first
    let is_active = state
        .identity
        .read()
        .as_ref()
        .is_some_and(|id| id.public_key == public_key);

    if is_active {
        // Drop the Stronghold keystore handle (no save needed — keys are immutable after create)
        keystore_handle.lock().take();

        // Send shutdown signal to sync and game services
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

        // Shut down voice engine
        {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                handle.engine.stop_capture();
                handle.engine.stop_playback();
            }
            *ve = None;
        }

        // Clean up user-specific DHT state (node stays alive)
        services::veilid_service::logout_cleanup(Some(&app), state.inner()).await;

        // Destroy all windows except login so labels are immediately freed
        for (label, window) in app.webview_windows() {
            if label != "login" {
                let _ = window.destroy();
            }
        }

        // Clear remaining per-session state
        *state.sync_shutdown_tx.write() = None;
        *state.game_detector.lock() = None;
    }

    // Delete from SQLite (CASCADE deletes all scoped data)
    let db = pool.inner().clone();
    let pk = public_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM identity WHERE public_key = ?1",
            rusqlite::params![pk],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Delete the Stronghold snapshot file
    StrongholdKeystore::delete_snapshot(&config_dir, &public_key)
        .map_err(|e| format!("failed to delete keystore: {e}"))?;

    tracing::info!(public_key = %public_key, "identity deleted");
    Ok(())
}

/// Load friends from `SQLite` into `AppState`, scoped to the given identity.
async fn load_friends_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let db = pool.clone();
    let ok = owner_key.to_string();
    let friend_rows = tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT f.public_key, f.display_name, f.nickname, f.dht_record_key, \
                 f.last_seen_at, f.local_conversation_key, f.remote_conversation_key, \
                 f.mailbox_dht_key, g.name AS group_name \
                 FROM friends f LEFT JOIN friend_groups g ON f.group_id = g.id \
                 WHERE f.owner_key = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok(FriendState {
                    public_key: db::get_str(row, "public_key"),
                    display_name: db::get_str(row, "display_name"),
                    nickname: db::get_str_opt(row, "nickname"),
                    status: UserStatus::Offline,
                    status_message: None,
                    game_info: None,
                    group: db::get_str_opt(row, "group_name"),
                    unread_count: 0,
                    dht_record_key: db::get_str_opt(row, "dht_record_key"),
                    last_seen_at: row.get::<_, Option<i64>>("last_seen_at").unwrap_or(None),
                    local_conversation_key: db::get_str_opt(row, "local_conversation_key"),
                    remote_conversation_key: db::get_str_opt(row, "remote_conversation_key"),
                    mailbox_dht_key: db::get_str_opt(row, "mailbox_dht_key"),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok::<Vec<FriendState>, String>(rows)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut friends = state.friends.write();
    for friend in friend_rows {
        let public_key = friend.public_key.clone();
        friends.insert(public_key, friend);
    }
    Ok(())
}

/// Load communities and channels from `SQLite` into `AppState`, scoped to the given identity.
#[allow(clippy::too_many_lines)]
async fn load_communities_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    use crate::state::RoleDefinition;

    let db = pool.clone();
    let ok = owner_key.to_string();
    let (community_rows, channel_rows, role_rows) = tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;

        let mut comm_stmt = conn
            .prepare(
                "SELECT id, name, description, my_role, my_role_ids, dht_record_key, dht_owner_keypair, \
                 my_pseudonym_key, mek_generation, server_route_blob, is_hosted \
                 FROM communities WHERE owner_key = ?1",
            )
            .map_err(|e| e.to_string())?;
        let communities = comm_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "id"),
                    db::get_str(row, "name"),
                    db::get_str_opt(row, "description"),
                    db::get_str(row, "my_role"),
                    db::get_str(row, "my_role_ids"),
                    db::get_str_opt(row, "dht_record_key"),
                    db::get_str_opt(row, "dht_owner_keypair"),
                    db::get_str_opt(row, "my_pseudonym_key"),
                    row.get::<_, i64>("mek_generation").unwrap_or(0).cast_unsigned(),
                    row.get::<_, Option<Vec<u8>>>("server_route_blob").unwrap_or(None),
                    row.get::<_, i64>("is_hosted").unwrap_or(0) != 0,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        // Load community roles
        let mut role_stmt = conn
            .prepare(
                "SELECT community_id, role_id, name, color, permissions, position, hoist, mentionable \
                 FROM community_roles WHERE owner_key = ?1 ORDER BY position",
            )
            .map_err(|e| e.to_string())?;
        let role_rows = role_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "community_id"),
                    row.get::<_, u32>("role_id").unwrap_or(0),
                    db::get_str(row, "name"),
                    row.get::<_, u32>("color").unwrap_or(0),
                    row.get::<_, i64>("permissions").unwrap_or(0).cast_unsigned(),
                    row.get::<_, i32>("position").unwrap_or(0),
                    row.get::<_, i32>("hoist").unwrap_or(0) != 0,
                    row.get::<_, i32>("mentionable").unwrap_or(0) != 0,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let mut chan_stmt = conn
            .prepare("SELECT id, community_id, name, channel_type FROM channels WHERE owner_key = ?1 ORDER BY sort_order")
            .map_err(|e| e.to_string())?;
        let channels = chan_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "id"),
                    db::get_str(row, "community_id"),
                    db::get_str(row, "name"),
                    db::get_str(row, "channel_type"),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok::<(Vec<_>, Vec<_>, Vec<_>), String>((communities, channels, role_rows))
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut communities = state.communities.write();
    for (community_id, name, description, my_role, my_role_ids_json, dht_record_key, dht_owner_keypair, my_pseudonym_key, mek_generation, server_route_blob, is_hosted) in &community_rows {
        let channels: Vec<ChannelInfo> = channel_rows
            .iter()
            .filter(|(_, cid, _, _)| cid == community_id)
            .map(|(id, _, ch_name, ch_type)| {
                let channel_type = match ch_type.as_str() {
                    "voice" => ChannelType::Voice,
                    _ => ChannelType::Text,
                };
                ChannelInfo {
                    id: id.clone(),
                    name: ch_name.clone(),
                    channel_type,
                    unread_count: 0,
                }
            })
            .collect();

        let my_role_ids: Vec<u32> =
            serde_json::from_str(my_role_ids_json).unwrap_or_else(|_| vec![0, 1]);

        let roles: Vec<RoleDefinition> = role_rows
            .iter()
            .filter(|(cid, ..)| cid == community_id)
            .map(|(_, role_id, rname, color, permissions, position, hoist, mentionable)| {
                RoleDefinition {
                    id: *role_id,
                    name: rname.clone(),
                    color: *color,
                    permissions: *permissions,
                    position: *position,
                    hoist: *hoist,
                    mentionable: *mentionable,
                }
            })
            .collect();

        let mut community = CommunityState {
            id: community_id.clone(),
            name: name.clone(),
            description: description.clone(),
            channels,
            my_role_ids,
            roles,
            my_role: Some(my_role.clone()),
            dht_record_key: dht_record_key.clone(),
            dht_owner_keypair: dht_owner_keypair.clone(),
            my_pseudonym_key: my_pseudonym_key.clone(),
            mek_generation: *mek_generation,
            server_route_blob: server_route_blob.clone(),
            is_hosted: *is_hosted,
        };
        // Recalculate display role from role definitions (DB value may be stale)
        community.my_role = Some(crate::state::display_role_name(&community.my_role_ids, &community.roles));
        communities.insert(community_id.clone(), community);
    }
    Ok(())
}

/// Re-derive pseudonym keys and load MEKs from Stronghold into `mek_cache`.
///
/// Called during login after communities are loaded from `SQLite`. For each
/// community, derives the pseudonym (deterministic from `identity_secret` + `community_id`)
/// and loads the MEK from Stronghold if stored.
///
/// For **hosted** (owned) communities where the MEK is missing from Stronghold
/// (e.g. communities created before MEK persistence was added), a fresh MEK is
/// regenerated and immediately persisted so subsequent restarts succeed.
fn restore_community_pseudonyms_and_meks(
    state: &SharedState,
    keystore_handle: &KeystoreHandle,
    secret_key: &[u8; 32],
) {
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
    use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
    use rekindle_crypto::Keychain as _;

    // Collect community IDs and is_hosted flags
    let community_info: Vec<(String, bool)> = {
        let communities = state.communities.read();
        communities
            .values()
            .map(|c| (c.id.clone(), c.is_hosted))
            .collect()
    };

    let mut pseudonym_updates: Vec<(String, String)> = Vec::new();
    let mut mek_updates: Vec<(String, MediaEncryptionKey)> = Vec::new();
    let mut regenerated_community_ids: Vec<String> = Vec::new();

    for (community_id, is_hosted) in &community_info {
        // Derive pseudonym
        let signing_key = derive_community_pseudonym(secret_key, community_id);
        let pseudonym_hex = hex::encode(signing_key.verifying_key().as_bytes());
        pseudonym_updates.push((community_id.clone(), pseudonym_hex));

        // Try to load MEK from Stronghold
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            let key_name = mek_key_name(community_id);
            match ks.load_key(VAULT_COMMUNITIES, &key_name) {
                Ok(Some(mek_bytes)) if mek_bytes.len() >= 40 => {
                    // MEK payload: generation (8 bytes LE) + key (32 bytes)
                    let generation =
                        u64::from_le_bytes(mek_bytes[..8].try_into().unwrap_or_default());
                    let key_bytes: [u8; 32] =
                        mek_bytes[8..40].try_into().unwrap_or_default();
                    let mek = MediaEncryptionKey::from_bytes(key_bytes, generation);
                    mek_updates.push((community_id.clone(), mek));
                }
                Ok(_) if *is_hosted => {
                    // Owned community with no MEK in Stronghold — regenerate.
                    // This handles communities created before MEK persistence was added.
                    tracing::warn!(
                        community = %community_id,
                        "MEK missing from Stronghold for hosted community — regenerating"
                    );
                    let mek = MediaEncryptionKey::generate(1);

                    // Persist immediately so the next restart finds it
                    let mut mek_payload = Vec::with_capacity(40);
                    mek_payload.extend_from_slice(&mek.generation().to_le_bytes());
                    mek_payload.extend_from_slice(mek.as_bytes());
                    if let Err(e) = ks.store_key(VAULT_COMMUNITIES, &key_name, &mek_payload) {
                        tracing::warn!(error = %e, community = %community_id, "failed to persist regenerated MEK");
                    } else if let Err(e) = ks.save() {
                        tracing::warn!(error = %e, community = %community_id, "failed to save Stronghold after MEK regeneration");
                    } else {
                        tracing::info!(community = %community_id, "regenerated MEK persisted to Stronghold");
                    }

                    mek_updates.push((community_id.clone(), mek));
                    regenerated_community_ids.push(community_id.clone());
                }
                Ok(_) => {
                    // Non-hosted community with missing MEK — user needs to
                    // re-join or wait for MEK delivery from the community server.
                    tracing::warn!(
                        community = %community_id,
                        "MEK missing from Stronghold for joined community — \
                         will be delivered when connecting to community server"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        community = %community_id,
                        error = %e,
                        "failed to load MEK from Stronghold"
                    );
                }
            }
        }
    }

    // Update communities with derived pseudonyms
    {
        let mut communities = state.communities.write();
        for (community_id, pseudonym_hex) in pseudonym_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                // Only set if not already stored in DB (DB value takes precedence)
                if c.my_pseudonym_key.is_none() {
                    c.my_pseudonym_key = Some(pseudonym_hex);
                }
            }
        }

        // Fix mek_generation for communities that had their MEK regenerated
        for community_id in &regenerated_community_ids {
            if let Some(c) = communities.get_mut(community_id) {
                c.mek_generation = 1;
            }
        }
    }

    // Load MEKs into cache
    {
        let mut mek_cache = state.mek_cache.lock();
        for (community_id, mek) in mek_updates {
            tracing::debug!(
                community = %community_id,
                generation = mek.generation(),
                "restored MEK from Stronghold"
            );
            mek_cache.insert(community_id, mek);
        }
    }
}

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
fn start_background_services(
    app: &tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    secret_key: &[u8; 32],
    dht_keys: DhtKeysConfig,
) {
    // Initialize Signal Protocol session manager (returns serialized PreKeyBundle)
    let prekey_bundle_bytes = initialize_signal_manager(state, secret_key);

    // Clear any stale background handles from a previous session
    state.background_handles.lock().clear();

    // Start game detection (only after login — avoids burning CPU before auth)
    let (game_shutdown_tx, game_shutdown_rx) = mpsc::channel::<()>(1);
    services::game_service::initialize(state, game_shutdown_tx);
    let game_app = app.clone();
    let game_state = Arc::clone(state);
    let game_handle = tauri::async_runtime::spawn(async move {
        services::game_service::start_game_detection(game_app, game_state, game_shutdown_rx).await;
    });

    // Store the game handle so logout can abort it
    state.background_handles.lock().push(game_handle);

    // The Veilid node is already running (started at app startup).
    // Just spawn sync + DHT publish as background tasks.
    spawn_login_services(
        app,
        state,
        pool.clone(),
        prekey_bundle_bytes,
        dht_keys,
    );
}

/// Background task: start sync service and DHT publish using the existing node.
///
/// The Veilid node and dispatch loop are already running (started at app startup).
/// This function only spawns user-specific services: sync and DHT publish.
fn spawn_login_services(
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

    // Create sync service shutdown channel
    let (sync_shutdown_tx, sync_shutdown_rx) = mpsc::channel::<()>(1);

    // Spawn the periodic sync service
    let sync_state = Arc::clone(state);
    let sync_pool = pool.clone();
    let sync_handle = tauri::async_runtime::spawn(async move {
        services::sync_service::start_sync_loop(sync_state, sync_pool, sync_shutdown_rx).await;
    });

    *state.sync_shutdown_tx.write() = Some(sync_shutdown_tx);

    // Spawn DHT publish as a background task
    let dht_handle = tauri::async_runtime::spawn(spawn_dht_publish(
        app.clone(),
        state.clone(),
        pool,
        prekey_bundle_bytes,
        dht_keys,
    ));

    // Spawn proactive route refresh loop (re-allocates our private route every 120s)
    let (route_refresh_shutdown_tx, route_refresh_shutdown_rx) = mpsc::channel::<()>(1);
    let route_refresh_app = app.clone();
    let route_refresh_state = Arc::clone(state);
    let route_refresh_handle = tauri::async_runtime::spawn(
        services::veilid_service::route_refresh_loop(
            route_refresh_app,
            route_refresh_state,
            route_refresh_shutdown_rx,
        ),
    );

    // Store sub-task handles so they can be aborted on logout
    {
        let mut handles = state.background_handles.lock();
        handles.push(sync_handle);
        handles.push(dht_handle);
        handles.push(route_refresh_handle);
    }

    // Keep the shutdown sender alive (dropped on logout via background_handles abort)
    drop(route_refresh_shutdown_tx);

    // Spawn community server process if user owns any communities
    maybe_spawn_server(app, state);

    // Start server health check loop (monitors and auto-restarts the server)
    if state.server_process.lock().is_some() {
        let (health_tx, health_rx) = tokio::sync::mpsc::channel(1);
        let health_state = Arc::clone(state);
        let health_app = app.clone();
        let health_handle = tauri::async_runtime::spawn(async move {
            services::server_health_service::server_health_loop(
                health_state,
                health_app,
                health_rx,
            )
            .await;
        });
        *state.server_health_shutdown_tx.write() = Some(health_tx);
        state.background_handles.lock().push(health_handle);
    }
}

/// Check if the current user owns any communities and spawn the community
/// server process if needed.
///
/// The server binary (`rekindle-server`) is a separate Veilid node that
/// hosts community DHT records, processes member RPCs, and keeps records alive.
/// It communicates with this process via a Unix socket IPC.
pub fn maybe_spawn_server(app: &tauri::AppHandle, state: &SharedState) {
    // Collect hosted communities' data for IPC commands (before any .await)
    // Tuple: (community_id, dht_key, keypair, name, creator_pseudonym, creator_display_name)
    let creator_display_name = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default();
    let hosted_communities: Vec<(String, String, String, String, String, String)> = {
        let communities = state.communities.read();
        communities
            .values()
            .filter(|c| c.is_hosted)
            .filter_map(|c| {
                let dht_key = c.dht_record_key.as_ref()?;
                let keypair = c.dht_owner_keypair.as_ref()?;
                let pseudonym = c.my_pseudonym_key.clone().unwrap_or_default();
                Some((
                    c.id.clone(),
                    dht_key.clone(),
                    keypair.clone(),
                    c.name.clone(),
                    pseudonym,
                    creator_display_name.clone(),
                ))
            })
            .collect()
    };

    if hosted_communities.is_empty() {
        tracing::debug!("user does not own any communities (or missing keypairs) — server not needed");
        return;
    }

    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "failed to get app data dir for server");
            return;
        }
    };

    // Look for the rekindle-server binary next to the current executable
    let server_binary = match std::env::current_exe() {
        Ok(exe) => exe.parent().map(|p| p.join("rekindle-server")).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to locate current exe for server binary");
            return;
        }
    };

    if !server_binary.exists() {
        tracing::info!(
            path = %server_binary.display(),
            "rekindle-server binary not found — community server will not be started. \
             This is expected during development; build with `cargo build -p rekindle-server`."
        );
        return;
    }

    let socket_path = std::env::temp_dir().join("rekindle-server.sock");

    // Kill any stale server process from a previous app session.
    // The old process may still hold the socket, blocking the new server.
    kill_stale_server(state, &socket_path);

    match std::process::Command::new(&server_binary)
        .arg("--storage-dir")
        .arg(data_dir.join("server"))
        .arg("--socket")
        .arg(&socket_path)
        .arg("--db")
        .arg(data_dir.join("server.db"))
        .spawn()
    {
        Ok(child) => {
            tracing::info!(
                pid = child.id(),
                socket = %socket_path.display(),
                "rekindle-server spawned"
            );
            *state.server_process.lock() = Some(child);

            // Send HostCommunity IPC commands in a background task
            let sp = socket_path.clone();
            let bg_state = Arc::clone(state);
            let handle = tauri::async_runtime::spawn(
                send_host_community_ipc(sp, hosted_communities),
            );
            bg_state.background_handles.lock().push(handle);
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to spawn rekindle-server");
        }
    }
}

/// Send `HostCommunity` IPC commands to the server for each owned community.
///
/// The server needs a moment to start up, so `host_community_blocking` retries
/// internally with backoff. The creator pseudonym key is passed so the server
/// can register the creator atomically during hosting.
async fn send_host_community_ipc(
    socket_path: std::path::PathBuf,
    communities: Vec<(String, String, String, String, String, String)>,
) {
    for (community_id, dht_key, keypair, name, pseudonym, display_name) in &communities {
        let sp = socket_path.clone();
        let cid = community_id.clone();
        let dk = dht_key.clone();
        let kp = keypair.clone();
        let nm = name.clone();
        let ps = pseudonym.clone();
        let dn = display_name.clone();
        let result = tokio::task::spawn_blocking(move || {
            crate::ipc_client::host_community_blocking(&sp, &cid, &dk, &kp, &nm, &ps, &dn, 10)
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(community = %community_id, error = %e, "failed to send HostCommunity IPC");
            }
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "HostCommunity IPC task panicked");
            }
        }
    }
}

/// Kill any stale `rekindle-server` process from a previous app session.
///
/// When the app quits unexpectedly (crash, force-quit), the server child
/// process may keep running. On next launch, `state.server_process` is `None`
/// (fresh state) but the old process still holds the Unix socket. This
/// function cleans up both the process and the socket file.
fn kill_stale_server(state: &SharedState, socket_path: &std::path::Path) {
    // First, try to kill the tracked child process (if any)
    {
        let mut proc = state.server_process.lock();
        if let Some(ref mut child) = *proc {
            let pid = child.id();
            tracing::info!(pid, "killing tracked server process before respawn");
            let _ = child.kill();
            let _ = child.wait();
            *proc = None;
        }
    }

    // Try to send a graceful Shutdown command to any server listening on the socket
    if socket_path.exists() {
        match crate::ipc_client::shutdown_server_blocking(socket_path) {
            Ok(()) => {
                tracing::info!("sent Shutdown to stale server on socket");
                // Give it a moment to exit
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                tracing::debug!(error = %e, "no server responded on socket (already dead)");
            }
        }
        // Remove the stale socket file so the new server can bind
        let _ = std::fs::remove_file(socket_path);
    }
}

/// Allocate a Veilid private route with retry.
///
/// Route allocation can fail transiently after the network becomes ready because
/// peerinfo may not have been published yet. We retry up to `max_attempts` times
/// with a 3-second delay between attempts.
async fn allocate_route_with_retry(app_handle: &tauri::AppHandle, state: &SharedState, max_attempts: u32) -> Option<Vec<u8>> {
    for attempt in 1..=max_attempts {
        let api = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.api.clone())
        }?;

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
                services::veilid_service::emit_network_status(app_handle, state);
                tracing::info!(attempt, "private route allocated");
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
///
/// Uses a `watch` channel to wait for network readiness — no TOCTOU race since
/// `watch::Receiver::changed()` returns immediately if the value was already set.
/// Route allocation is done here (after readiness) instead of in `initialize_node()`
/// where the network isn't connected yet.
async fn spawn_dht_publish(
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
        tracing::warn!("failed to allocate private route after retries — peers won't be able to message us");
    }

    // Create or open mailbox DHT record
    if let Err(e) = publish_mailbox(&state, &pool, dht_keys.mailbox_dht_key.as_ref(), route_blob.as_deref()).await {
        tracing::warn!(error = %e, "mailbox publish failed");
    }

    tracing::info!("public internet ready — publishing profile to DHT");

    if let Err(e) = publish_profile_to_dht(
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

    if let Err(e) =
        publish_friend_list_to_dht(&state, &pool, dht_keys.existing_friend_list_key, dht_keys.friend_list_owner_keypair)
            .await
    {
        tracing::warn!(error = %e, "DHT friend list publish failed — will retry on next sync");
    }

    // Immediate friend sync now that network is up
    if let Err(e) = services::sync_service::sync_friends_now(&state).await {
        tracing::warn!(error = %e, "immediate friend sync failed");
    }

    // Publish account record (Phase 3)
    if let Err(e) = publish_account_to_dht(
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

/// Create or open the mailbox DHT record and publish the current route blob.
///
/// The mailbox uses the identity Ed25519 keypair as the DHT record owner,
/// making the record key deterministic and permanent for this identity.
async fn publish_mailbox(
    state: &SharedState,
    pool: &DbPool,
    existing_mailbox_key: Option<&String>,
    route_blob: Option<&[u8]>,
) -> Result<(), String> {
    let (public_key, secret_bytes) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set before mailbox publish")?;
        let secret = state.identity_secret.lock();
        let secret = *secret.as_ref().ok_or("identity secret not available")?;
        (id.public_key.clone(), secret)
    };

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized before mailbox publish")?;
        nh.routing_context.clone()
    };

    // Build a Veilid KeyPair from our Ed25519 identity keys.
    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let pub_bytes = identity.public_key_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    let veilid_keypair = veilid_core::KeyPair::new_from_parts(veilid_pubkey, bare_secret);

    let mailbox_key = if let Some(existing_key) = existing_mailbox_key {
        // Re-open existing mailbox with write access
        match rekindle_protocol::dht::mailbox::open_mailbox_writable(
            &routing_context,
            existing_key,
            veilid_keypair.clone(),
        )
        .await
        {
            Ok(()) => {
                tracing::info!(key = %existing_key, "reopened existing mailbox");
                existing_key.clone()
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to reopen mailbox — creating new one");
                rekindle_protocol::dht::mailbox::create_mailbox(&routing_context, veilid_keypair)
                    .await
                    .map_err(|e| format!("create mailbox: {e}"))?
            }
        }
    } else {
        rekindle_protocol::dht::mailbox::create_mailbox(&routing_context, veilid_keypair)
            .await
            .map_err(|e| format!("create mailbox: {e}"))?
    };

    // Write route blob to mailbox subkey 0
    if let Some(blob) = route_blob {
        if !blob.is_empty() {
            rekindle_protocol::dht::mailbox::update_mailbox_route(&routing_context, &mailbox_key, blob)
                .await
                .map_err(|e| format!("update mailbox route: {e}"))?;
        }
    }

    // Store on NodeHandle
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.mailbox_dht_key = Some(mailbox_key.clone());
        }
    }

    // Track the open record
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.track_open_record(mailbox_key.clone());
        }
    }

    // Persist to SQLite
    let db = pool.clone();
    let pk = public_key;
    let mk = mailbox_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET mailbox_dht_key = ?1 WHERE public_key = ?2",
            rusqlite::params![mk, pk],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(mailbox_key = %mailbox_key, "mailbox published to DHT");
    Ok(())
}

/// Try to open an existing profile DHT record and update all subkeys.
///
/// If `owner_keypair` is provided, the record is opened with write access.
/// Returns `Ok(())` if the record was opened and ALL subkeys were written successfully.
/// Returns `Err` if the open failed OR any write failed (e.g. "value is not writable"
/// when the owner keypair is missing or incorrect). The caller should fall back to
/// creating a new record in the `Err` case.
async fn try_update_existing_profile(
    dht: &rekindle_protocol::dht::DHTManager,
    existing_key: &str,
    display_name: &str,
    status_message: &str,
    prekey_bundle: &[u8],
    route_blob: &[u8],
    owner_keypair: Option<veilid_core::KeyPair>,
) -> Result<(), String> {
    let has_keypair = owner_keypair.is_some();
    if let Some(keypair) = owner_keypair {
        dht.open_record_writable(existing_key, keypair)
            .await
            .map_err(|e| format!("open writable: {e}"))?;
    } else {
        // No keypair available — open read-only and hope for the best
        // (will likely fail on the first write with "value is not writable")
        dht.open_record(existing_key)
            .await
            .map_err(|e| format!("open: {e}"))?;
    }
    tracing::info!(key = %existing_key, has_keypair, "reusing existing DHT profile record");
    rekindle_protocol::dht::profile::update_subkey(
        dht, existing_key, 0, display_name.as_bytes().to_vec(),
    )
    .await
    .map_err(|e| format!("display name: {e}"))?;
    rekindle_protocol::dht::profile::update_subkey(
        dht, existing_key, 1, status_message.as_bytes().to_vec(),
    )
    .await
    .map_err(|e| format!("status message: {e}"))?;
    rekindle_protocol::dht::profile::update_subkey(
        dht, existing_key, 2, vec![0], // online
    )
    .await
    .map_err(|e| format!("status: {e}"))?;
    rekindle_protocol::dht::profile::update_subkey(
        dht, existing_key, 5, prekey_bundle.to_vec(),
    )
    .await
    .map_err(|e| format!("prekey bundle: {e}"))?;
    rekindle_protocol::dht::profile::update_subkey(
        dht, existing_key, 6, route_blob.to_vec(),
    )
    .await
    .map_err(|e| format!("route blob: {e}"))?;
    Ok(())
}

/// Create (or reuse) a DHT profile record and publish identity data after login.
///
/// Publishes display name (subkey 0), status message (subkey 1), status (subkey 2),
/// `PreKeyBundle` (subkey 5), and route blob (subkey 6) so that friends can discover
/// our presence and establish encrypted sessions.
///
/// If `existing_dht_key` is provided (from a previous session stored in `SQLite`),
/// attempts to open and reuse that record. Falls back to creating a new one if the
/// open or any write fails (e.g., record expired, or "value is not writable" when
/// the owner keypair isn't available).
async fn publish_profile_to_dht(
    state: &SharedState,
    pool: &DbPool,
    prekey_bundle_bytes: Option<Vec<u8>>,
    existing_dht_key: Option<String>,
    dht_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    // Extract identity data (clone out before .await — parking_lot guards are !Send)
    let (public_key, display_name, status_message) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set before DHT publish")?;
        (id.public_key.clone(), id.display_name.clone(), id.status_message.clone())
    };

    // Extract route blob from node handle
    let route_blob = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized before DHT publish")?;
        nh.route_blob.clone().unwrap_or_default()
    };

    // Clone the routing context out of the DHT manager before .await
    // (parking_lot guards are !Send — must drop before any await point)
    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized before DHT profile creation")?;
        nh.routing_context.clone()
    };
    let temp_dht = rekindle_protocol::dht::DHTManager::new(routing_context);

    let bundle = prekey_bundle_bytes.as_deref().unwrap_or(&[]);

    // Parse the stored owner keypair (if any) for writable DHT access.
    let owner_keypair: Option<veilid_core::KeyPair> = dht_owner_keypair_str.as_ref().and_then(|s| {
        s.parse().map_err(|e| {
            tracing::warn!(error = %e, "failed to parse stored DHT owner keypair — will create new record");
            e
        }).ok()
    });

    // Try to reuse existing DHT record with the owner keypair for write access.
    // If no keypair is stored, writes will fail and we fall back to creating fresh.
    let (profile_key, new_keypair) = if let Some(ref existing_key) = existing_dht_key {
        match try_update_existing_profile(
            &temp_dht,
            existing_key,
            &display_name,
            &status_message,
            bundle,
            &route_blob,
            owner_keypair.clone(),
        )
        .await
        {
            Ok(()) => (existing_key.clone(), None),
            Err(e) => {
                tracing::warn!(
                    key = %existing_key, error = %e,
                    "failed to reuse existing DHT profile — creating new one"
                );
                rekindle_protocol::dht::profile::create_profile(
                    &temp_dht, &display_name, &status_message, bundle, &route_blob,
                )
                .await
                .map_err(|e| format!("failed to create DHT profile: {e}"))?
            }
        }
    } else {
        rekindle_protocol::dht::profile::create_profile(
            &temp_dht, &display_name, &status_message, bundle, &route_blob,
        )
        .await
        .map_err(|e| format!("failed to create DHT profile: {e}"))?
    };

    // Determine the effective owner keypair: either the new one (just created) or the existing one (reused)
    let effective_keypair = new_keypair.clone().or(owner_keypair);

    // Store the profile DHT key and owner keypair on both the node handle and the DHT manager
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.profile_dht_key = Some(profile_key.clone());
            nh.profile_owner_keypair.clone_from(&effective_keypair);
        }
    }
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.manager.profile_key = Some(profile_key.clone());
            mgr.track_open_record(profile_key.clone());
        }
    }

    // Persist profile_dht_key and owner keypair to SQLite so they survive restarts.
    // The keypair string is stored via KeyPair's Display impl and loaded via FromStr.
    let keypair_str = new_keypair.map(|kp| kp.to_string());
    let has_new_keypair = keypair_str.is_some();
    let db = pool.clone();
    let pk = public_key;
    let dht_key = profile_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET dht_record_key = ?1, dht_owner_keypair = COALESCE(?3, dht_owner_keypair) \
             WHERE public_key = ?2",
            rusqlite::params![dht_key, pk, keypair_str],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(
        profile_key = %profile_key,
        has_prekey_bundle = prekey_bundle_bytes.is_some(),
        has_route_blob = !route_blob.is_empty(),
        new_keypair_stored = has_new_keypair,
        "published profile to DHT"
    );

    Ok(())
}

/// Create (or reuse) a DHT friend list record after login.
///
/// If `existing_friend_list_key` is provided, attempts to open it. Falls back to
/// creating a new one if the open fails. Persists the key to `SQLite` and sets it
/// on both `NodeHandle` and `DHTManagerHandle`.
async fn publish_friend_list_to_dht(
    state: &SharedState,
    pool: &DbPool,
    existing_friend_list_key: Option<String>,
    friend_list_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let public_key = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set before friend list publish")?;
        id.public_key.clone()
    };

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized before friend list creation")?;
        nh.routing_context.clone()
    };
    let temp_dht = rekindle_protocol::dht::DHTManager::new(routing_context);

    // Parse the stored owner keypair (if any) for writable DHT access.
    let owner_keypair: Option<veilid_core::KeyPair> = friend_list_owner_keypair_str.as_ref().and_then(|s| {
        s.parse().map_err(|e| {
            tracing::warn!(error = %e, "failed to parse stored friend list owner keypair — will create new record");
            e
        }).ok()
    });

    // Try to reuse existing record with writable access, fall back to creating new one
    let has_keypair = owner_keypair.is_some();
    let (friend_list_key, new_keypair) = if let Some(ref existing_key) = existing_friend_list_key {
        let open_result = if let Some(ref keypair) = owner_keypair {
            temp_dht.open_record_writable(existing_key, keypair.clone()).await
        } else {
            temp_dht.open_record(existing_key).await
        };
        match open_result {
            Ok(()) => {
                tracing::info!(key = %existing_key, has_keypair, "reusing existing DHT friend list record");
                (existing_key.clone(), None)
            }
            Err(e) => {
                tracing::warn!(
                    key = %existing_key, error = %e,
                    "failed to open existing friend list record — creating new one"
                );
                rekindle_protocol::dht::friends::create_friend_list(&temp_dht)
                    .await
                    .map_err(|e| format!("failed to create friend list DHT record: {e}"))?
            }
        }
    } else {
        rekindle_protocol::dht::friends::create_friend_list(&temp_dht)
            .await
            .map_err(|e| format!("failed to create friend list DHT record: {e}"))?
    };

    // Determine the effective owner keypair: either the existing one (reused) or the new one (just created)
    let effective_keypair = new_keypair.clone().or(owner_keypair);

    // Store the friend list DHT key and owner keypair on NodeHandle and DHTManagerHandle
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.friend_list_dht_key = Some(friend_list_key.clone());
            nh.friend_list_owner_keypair = effective_keypair;
        }
    }
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.manager.friend_list_key = Some(friend_list_key.clone());
            mgr.track_open_record(friend_list_key.clone());
        }
    }

    // Persist friend_list_dht_key and owner keypair to SQLite so they survive restarts
    let keypair_str = new_keypair.map(|kp| kp.to_string());
    let has_new_keypair = keypair_str.is_some();
    let db = pool.clone();
    let pk = public_key;
    let fl_key = friend_list_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET friend_list_dht_key = ?1, \
             friend_list_owner_keypair = COALESCE(?3, friend_list_owner_keypair) \
             WHERE public_key = ?2",
            rusqlite::params![fl_key, pk, keypair_str],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(
        friend_list_key = %friend_list_key,
        new_keypair_stored = has_new_keypair,
        "published friend list to DHT"
    );
    Ok(())
}

/// Create a fresh account DHT record (helper to reduce duplication).
async fn create_fresh_account_record(
    routing_context: &veilid_core::RoutingContext,
    encryption_key: rekindle_crypto::DhtRecordKey,
    display_name: &str,
    status_message: &str,
) -> Result<(String, Option<veilid_core::KeyPair>), String> {
    let (record, kp) = rekindle_protocol::dht::account::AccountRecord::create(
        routing_context,
        encryption_key,
        display_name,
        status_message,
    )
    .await
    .map_err(|e| format!("create account record: {e}"))?;
    Ok((record.record_key(), Some(kp)))
}

/// Store an account DHT key on `NodeHandle` and track records in `DHTManagerHandle`.
fn store_account_key_on_handles(state: &SharedState, account_key: &str, record_keys: Vec<String>) {
    if let Some(ref mut nh) = *state.node.write() {
        nh.account_dht_key = Some(account_key.to_string());
    }
    let mut dht_mgr = state.dht_manager.write();
    if let Some(ref mut mgr) = *dht_mgr {
        for k in record_keys {
            mgr.track_open_record(k);
        }
    }
}

/// Persist account DHT key and owner keypair to `SQLite`.
async fn persist_account_key_to_db(
    pool: &DbPool,
    public_key: &str,
    account_key: &str,
    new_keypair: Option<veilid_core::KeyPair>,
) -> Result<(), String> {
    let keypair_str = new_keypair.map(|kp| kp.to_string());
    let db = pool.clone();
    let pk = public_key.to_string();
    let ak = account_key.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET account_dht_key = ?1, \
             account_owner_keypair = COALESCE(?3, account_owner_keypair) \
             WHERE public_key = ?2",
            rusqlite::params![ak, pk, keypair_str],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Create (or reopen) the private Account DHT record after login.
///
/// The account record is encrypted with a key derived from the identity's Ed25519 secret.
/// It holds pointers to contact list, chat list, and invitation list `DHTShortArray`s.
async fn publish_account_to_dht(
    state: &SharedState,
    pool: &DbPool,
    existing_account_key: Option<String>,
    account_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let (public_key, display_name, status_message) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set before account publish")?;
        (id.public_key.clone(), id.display_name.clone(), id.status_message.clone())
    };

    let secret_bytes = state
        .identity_secret
        .lock()
        .ok_or("identity secret not available for account key derivation")?;
    let encryption_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized before account publish")?;
        nh.routing_context.clone()
    };

    let owner_keypair: Option<veilid_core::KeyPair> = account_owner_keypair_str.as_ref().and_then(|s| {
        s.parse().map_err(|e| {
            tracing::warn!(error = %e, "failed to parse stored account owner keypair");
            e
        }).ok()
    });

    let (account_key, new_keypair) = if let Some(ref existing_key) = existing_account_key {
        if let Some(keypair) = owner_keypair {
            match rekindle_protocol::dht::account::AccountRecord::open(
                &routing_context,
                existing_key,
                keypair,
                encryption_key,
            )
            .await
            {
                Ok(record) => {
                    tracing::info!(key = %existing_key, "reusing existing account DHT record");
                    store_account_key_on_handles(state, &record.record_key(), record.all_record_keys());
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        key = %existing_key, error = %e,
                        "failed to open existing account record — creating new one"
                    );
                    let enc_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
                    create_fresh_account_record(&routing_context, enc_key, &display_name, &status_message).await?
                }
            }
        } else {
            tracing::warn!("no account owner keypair — creating new account record");
            let enc_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
            create_fresh_account_record(&routing_context, enc_key, &display_name, &status_message).await?
        }
    } else {
        create_fresh_account_record(&routing_context, encryption_key, &display_name, &status_message).await?
    };

    store_account_key_on_handles(state, &account_key, vec![account_key.clone()]);
    persist_account_key_to_db(pool, &public_key, &account_key, new_keypair).await?;

    tracing::info!(account_key = %account_key, "published account record to DHT");
    Ok(())
}

/// Create a conversation record for a specific friend.
///
/// Called when establishing a new contact. Creates a `ConversationRecord`,
/// and persists the keys to `SQLite`.
pub async fn create_conversation_for_friend(
    state: &SharedState,
    pool: &DbPool,
    friend_public_key: &str,
) -> Result<String, String> {
    let secret_bytes = state
        .identity_secret
        .lock()
        .ok_or("identity secret not available")?;

    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let my_x25519_secret = identity.to_x25519_secret();

    // Parse friend's Ed25519 public key and derive X25519 public key.
    // Uses Edwards→Montgomery birational map on the PUBLIC key (NOT from_secret_bytes).
    let friend_ed_bytes = hex::decode(friend_public_key)
        .map_err(|e| format!("invalid friend public key hex: {e}"))?;
    let friend_ed_bytes: [u8; 32] = friend_ed_bytes
        .try_into()
        .map_err(|_| "friend public key must be 32 bytes".to_string())?;
    let friend_x25519_public = rekindle_crypto::Identity::peer_ed25519_to_x25519(&friend_ed_bytes)
        .map_err(|e| format!("failed to convert friend key to X25519: {e}"))?;

    let encryption_key =
        rekindle_crypto::DhtRecordKey::derive_conversation_key(&my_x25519_secret, &friend_x25519_public);

    let (display_name, status_message, owner_key) = {
        let id = state.identity.read();
        let id = id.as_ref().ok_or("not logged in")?;
        (id.display_name.clone(), id.status_message.clone(), id.public_key.clone())
    };

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        nh.routing_context.clone()
    };

    let route_blob = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| nh.route_blob.clone()).unwrap_or_default()
    };

    let profile = rekindle_protocol::capnp_codec::identity::UserProfile {
        display_name: display_name.clone(),
        status_message,
        status: 0,
        avatar_hash: Vec::new(),
        game_status: None,
    };

    let prekey = rekindle_protocol::capnp_codec::identity::PreKeyBundle {
        identity_key: identity.public_key_bytes().to_vec(),
        signed_pre_key: Vec::new(),
        signed_pre_key_sig: Vec::new(),
        one_time_pre_key: Vec::new(),
        registration_id: 0,
    };

    let (record, _keypair) = rekindle_protocol::dht::conversation::ConversationRecord::create(
        &routing_context,
        encryption_key,
        &identity.public_key_bytes(),
        &profile,
        &route_blob,
        &prekey,
    )
    .await
    .map_err(|e| format!("create conversation: {e}"))?;

    let conversation_key = record.record_key();

    // Track all record keys (parent + message log)
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            for k in record.all_record_keys() {
                mgr.track_open_record(k);
            }
        }
    }

    // Persist to SQLite
    let db = pool.clone();
    let ok = owner_key;
    let fpk = friend_public_key.to_string();
    let ck = conversation_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET local_conversation_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![ck, ok, fpk],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Update in-memory state
    {
        let mut friends = state.friends.write();
        if let Some(f) = friends.get_mut(friend_public_key) {
            f.local_conversation_key = Some(conversation_key.clone());
        }
    }

    tracing::info!(
        friend = %friend_public_key,
        conversation_key = %conversation_key,
        "created conversation record for friend"
    );

    Ok(conversation_key)
}

/// Initialize the Signal Protocol session manager with the identity key.
///
/// Creates in-memory stores for identity, prekeys, and sessions, then
/// generates an initial `PreKeyBundle` for DHT publication.
///
/// Returns the serialized `PreKeyBundle` bytes if generation succeeded,
/// so the caller can publish them to DHT profile subkey 5.
fn initialize_signal_manager(
    state: &SharedState,
    secret_key: &[u8; 32],
) -> Option<Vec<u8>> {
    use rekindle_crypto::signal::{
        MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore, SignalSessionManager,
    };

    // Derive the X25519 key pair from the Ed25519 secret key for X3DH
    let identity = rekindle_crypto::Identity::from_secret_bytes(secret_key);
    let x25519_secret = identity.to_x25519_secret();
    let x25519_public = identity.to_x25519_public();

    // Use X25519 key bytes for the Signal identity store (X3DH uses X25519)
    let identity_private = x25519_secret.to_bytes().to_vec();
    let identity_public = x25519_public.as_bytes().to_vec();

    // Registration ID — derive deterministically from the public key so it's stable
    let pub_bytes = identity.public_key_bytes();
    let registration_id = u32::from_le_bytes([pub_bytes[0], pub_bytes[1], pub_bytes[2], pub_bytes[3]]);

    let identity_store = MemoryIdentityStore::new(identity_private, identity_public, registration_id);
    let prekey_store = MemoryPreKeyStore::new();
    let session_store = MemorySessionStore::new();

    let manager = SignalSessionManager::new(
        Box::new(identity_store),
        Box::new(prekey_store),
        Box::new(session_store),
    );

    // Generate initial PreKeyBundle (signed prekey #1, one-time prekey #1)
    let bundle_bytes = match manager.generate_prekey_bundle(1, Some(1)) {
        Ok(bundle) => {
            tracing::info!(
                registration_id = bundle.registration_id,
                "Signal session manager initialized with PreKeyBundle"
            );
            match serde_json::to_vec(&bundle) {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize PreKeyBundle for DHT publication");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to generate initial PreKeyBundle — sessions will still work via respond_to_session");
            None
        }
    };

    *state.signal_manager.lock() = Some(SignalManagerHandle { manager });

    // Store the Ed25519 secret key bytes so message_service can sign envelopes
    *state.identity_secret.lock() = Some(*secret_key);

    bundle_bytes
}
