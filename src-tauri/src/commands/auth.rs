use std::sync::Arc;

use rekindle_crypto::keychain::{KEY_ED25519_PRIVATE, VAULT_IDENTITY};
use rekindle_crypto::Keychain as _;
use rusqlite::OptionalExtension as _;
use serde::{Deserialize, Serialize};
use tauri::{Emitter as _, Manager as _, State};
use tokio::sync::mpsc;

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::keystore::{KeystoreHandle, StrongholdKeystore};
use crate::services;
use crate::state::{
    CategoryInfo, ChannelInfo, ChannelType, CommunityState, FriendState, IdentityState,
    SharedState, SignalManagerHandle, UserStatus,
};
use crate::state_helpers;

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
    std::fs::create_dir_all(config_dir).map_err(|e| format!("failed to create config dir: {e}"))?;

    let identity = rekindle_crypto::Identity::generate();
    let public_key = identity.public_key_hex();
    let secret_bytes = *identity.secret_key_bytes();
    let display_name = display_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("User_{}", &public_key[..8]));
    let now = db::timestamp_now();

    // Initialize per-identity Stronghold and store the private key
    let keystore = StrongholdKeystore::initialize_for_identity(config_dir, &public_key, passphrase)
        .map_err(|e| crate::keystore::map_stronghold_error(&e))?;
    keystore
        .store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &secret_bytes)
        .map_err(|e| e.to_string())?;
    keystore.save().map_err(|e| e.to_string())?;

    // Keep the keystore unlocked for the session
    *keystore_handle.lock() = Some(keystore);

    // Persist identity to SQLite (alongside any existing identities)
    let pk = public_key.clone();
    let dn = display_name.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO identity (public_key, display_name, created_at) VALUES (?, ?, ?)",
            rusqlite::params![pk, dn, now],
        )?;
        Ok(())
    })
    .await?;

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
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

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
    start_background_services(
        &app,
        state.inner(),
        pool.inner(),
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
    let pk_query = public_key.to_string();
    let (display_name, dht_cols) = db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT display_name, dht_record_key, friend_list_dht_key, \
                     dht_owner_keypair, friend_list_owner_keypair, \
                     account_dht_key, account_owner_keypair, mailbox_dht_key \
                     FROM identity WHERE public_key = ?1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![pk_query], |row| {
                Ok((
                    row.get::<_, String>("display_name").unwrap_or_default(),
                    IdentityDhtColumns {
                        existing_dht_key: row.get::<_, Option<String>>("dht_record_key")?,
                        existing_friend_list_key: row
                            .get::<_, Option<String>>("friend_list_dht_key")?,
                        dht_owner_keypair: row.get::<_, Option<String>>("dht_owner_keypair")?,
                        friend_list_owner_keypair: row
                            .get::<_, Option<String>>("friend_list_owner_keypair")?,
                        account_dht_key: row.get::<_, Option<String>>("account_dht_key")?,
                        account_owner_keypair: row
                            .get::<_, Option<String>>("account_owner_keypair")?,
                        mailbox_dht_key: row.get::<_, Option<String>>("mailbox_dht_key")?,
                    },
                ))
            })
            .optional()?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
        Ok(row)
    })
    .await
    .map_err(|e| {
        if e.contains("Query returned no rows") {
            "no identity found — please create one first".to_string()
        } else {
            e
        }
    })?;

    // Unlock per-identity Stronghold with passphrase and load private key
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
        return Err("Wrong passphrase — decrypted key does not match stored identity".to_string());
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
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    let (result, secret_key, dht_cols) = login_core(
        &config_dir,
        &public_key,
        &passphrase,
        state.inner(),
        pool.inner(),
        keystore_handle.inner(),
    )
    .await?;

    // Spawn background services — waits for route restoration so communities
    // are usable immediately after login returns.
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

    // Wait for Veilid network + sync friends so hydrateState() gets real statuses.
    // The node starts at app launch, so this is usually instant on re-login
    // and 5-20s on cold start. The frontend shows a loading spinner.
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
            if let Err(e) = services::sync_service::sync_friends_now(state.inner(), &app).await {
                tracing::warn!(error = %e, "login-time friend sync failed");
            }
        } else {
            tracing::warn!("network not ready within 20s — buddy list will use fallback sync");
        }
    }

    // Replay any deep link that arrived before authentication
    crate::deep_links::emit_pending_deep_link(&app);

    Ok(result)
}

/// Get the current identity state.
///
/// Used by newly opened windows to hydrate their local auth state
/// from the shared Rust backend (each webview has isolated JS context).
#[tauri::command]
pub async fn get_identity(state: State<'_, SharedState>) -> Result<Option<LoginResult>, String> {
    let identity = state.identity.read();
    Ok(identity.as_ref().map(|id| LoginResult {
        public_key: id.public_key.clone(),
        display_name: id.display_name.clone(),
    }))
}

/// Shut down the voice engine: signal all tokio loops, await them, then stop devices.
use crate::commands::voice::{shutdown_voice, VoiceShutdownOpts};

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

    shutdown_voice(state.inner(), &VoiceShutdownOpts::FULL).await;

    // Signal route refresh loop shutdown (stored separately from background_handles)
    {
        let tx = state.route_refresh_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // Shut down idle service
    {
        let tx = state.idle_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }
    *state.pre_away_status.write() = None;

    // Shut down heartbeat service
    {
        let tx = state.heartbeat_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // Publish Offline to DHT BEFORE logout_cleanup clears the owner keypair
    {
        if state_helpers::identity_status(state.inner()) != Some(UserStatus::Offline) {
            if let Err(e) =
                services::presence_service::publish_status(state.inner(), UserStatus::Offline).await
            {
                tracing::warn!(error = %e, "failed to publish offline status on logout");
            }
        }
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
    services::veilid::logout_cleanup(Some(&app), &state).await;

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
pub async fn list_identities(pool: State<'_, DbPool>) -> Result<Vec<IdentitySummary>, String> {
    db_call(pool.inner(), move |conn| {
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
                    public_key: db::get_str(row, "public_key"),
                    display_name: row.get::<_, String>("display_name").unwrap_or_default(),
                    created_at: db::get_i64(row, "created_at"),
                    has_avatar: avatar_base64.is_some(),
                    avatar_base64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
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
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;

    // Verify passphrase by attempting to open the identity's Stronghold
    StrongholdKeystore::initialize_for_identity(&config_dir, &public_key, &passphrase)
        .map_err(|e| crate::keystore::map_stronghold_error(&e))?;

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

        // Shut down voice engine (signal tokio loops, await them, then stop devices)
        shutdown_voice(state.inner(), &VoiceShutdownOpts::FULL).await;

        // Signal route refresh loop shutdown
        {
            let tx = state.route_refresh_shutdown_tx.write().take();
            if let Some(tx) = tx {
                let _ = tx.send(()).await;
            }
        }

        // Shut down idle service
        {
            let tx = state.idle_shutdown_tx.write().take();
            if let Some(tx) = tx {
                let _ = tx.send(()).await;
            }
        }
        *state.pre_away_status.write() = None;

        // Publish Offline before cleanup
        {
            if state_helpers::identity_status(state.inner()) != Some(UserStatus::Offline) {
                let _ =
                    services::presence_service::publish_status(state.inner(), UserStatus::Offline)
                        .await;
            }
        }

        // Clean up user-specific DHT state (node stays alive)
        services::veilid::logout_cleanup(Some(&app), state.inner()).await;

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
    let pk = public_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM identity WHERE public_key = ?1",
            rusqlite::params![pk],
        )?;
        Ok(())
    })
    .await?;

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
    let ok = owner_key.to_string();
    let friend_rows = db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT f.public_key, f.display_name, f.nickname, f.dht_record_key, \
                 f.last_seen_at, f.local_conversation_key, f.remote_conversation_key, \
                 f.mailbox_dht_key, f.friendship_state, g.name AS group_name \
                 FROM friends f LEFT JOIN friend_groups g ON f.group_id = g.id \
                 WHERE f.owner_key = ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![ok], |row| {
                let fs_str: String = row
                    .get::<_, String>("friendship_state")
                    .unwrap_or_else(|_| "accepted".to_string());
                let friendship_state = match fs_str.as_str() {
                    "pending_out" => crate::state::FriendshipState::PendingOut,
                    _ => crate::state::FriendshipState::Accepted,
                };
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
                    last_heartbeat_at: None,
                    friendship_state,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await?;

    let mut friends = state.friends.write();
    for friend in friend_rows {
        let public_key = friend.public_key.clone();
        friends.insert(public_key, friend);
    }
    Ok(())
}

/// Load communities and channels from `SQLite` into `AppState`, scoped to the given identity.
async fn load_communities_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    use crate::state::RoleDefinition;

    let ok = owner_key.to_string();
    let (community_rows, channel_rows, role_rows, category_rows, member_key_rows, event_rsvp_rows) = db_call(pool, move |conn| {
        let mut comm_stmt = conn
            .prepare(
                "SELECT c.id, c.name, c.description, c.my_role, c.my_role_ids, c.dht_owner_keypair, \
                 c.my_pseudonym_key, c.mek_generation, c.member_registry_key, c.my_subkey_index, \
                 COALESCE(cm.onboarding_complete, 0) \
                 FROM communities c \
                 LEFT JOIN community_members cm \
                   ON cm.owner_key = c.owner_key \
                  AND cm.community_id = c.id \
                  AND cm.pseudonym_key = c.my_pseudonym_key \
                 WHERE c.owner_key = ?1",
            )?;
        let communities = comm_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "id"),
                    db::get_str(row, "name"),
                    db::get_str_opt(row, "description"),
                    db::get_str(row, "my_role"),
                    db::get_str(row, "my_role_ids"),
                    db::get_str_opt(row, "dht_owner_keypair"),
                    db::get_str_opt(row, "my_pseudonym_key"),
                    row.get::<_, i64>("mek_generation").unwrap_or(0).cast_unsigned(),
                    db::get_str_opt(row, "member_registry_key"),
                    row.get::<_, Option<i64>>("my_subkey_index").unwrap_or(None).map(|v| u32::try_from(v).unwrap_or(0)),
                    row.get::<_, i64>(10).unwrap_or(0) != 0,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load community roles
        let mut role_stmt = conn
            .prepare(
                "SELECT community_id, role_id, name, color, permissions, position, hoist, mentionable, self_assignable \
                 FROM community_roles WHERE owner_key = ?1 ORDER BY position",
            )?;
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
                    row.get::<_, i32>("self_assignable").unwrap_or(0) != 0,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load community categories
        let mut cat_stmt = conn
            .prepare(
                "SELECT community_id, id, name, sort_order \
                 FROM community_categories WHERE owner_key = ?1 ORDER BY sort_order",
            )?;
        let category_rows = cat_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "community_id"),
                    db::get_str(row, "id"),
                    db::get_str(row, "name"),
                    row.get::<_, i32>("sort_order").unwrap_or(0),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut chan_stmt = conn.prepare(
            "SELECT ch.id, ch.community_id, ch.name, ch.channel_type, ch.category_id, ch.topic, \
                    ch.slowmode_seconds, ch.nsfw, ch.message_record_key, ch.mek_generation, \
                    ch.log_key, ch.my_sequence, COALESCE(np.level, 0) AS notification_level \
             FROM channels ch
             LEFT JOIN notification_preferences np
               ON np.owner_key = ch.owner_key
              AND np.community_id = ch.community_id
              AND np.channel_id = ch.id
             WHERE ch.owner_key = ?1
             ORDER BY ch.sort_order",
        )?;
        let channels = chan_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "id"),
                    db::get_str(row, "community_id"),
                    db::get_str(row, "name"),
                    row.get::<_, ChannelType>("channel_type")?,
                    db::get_str_opt(row, "category_id"),
                    db::get_str(row, "topic"),
                    row.get::<_, Option<i64>>("slowmode_seconds").unwrap_or(None).map(|v| u32::try_from(v).unwrap_or(0)),
                    row.get::<_, i64>("nsfw").unwrap_or(0) != 0,
                    db::get_str_opt(row, "message_record_key"),
                    row.get::<_, i64>("mek_generation").unwrap_or(0).cast_unsigned(),
                    db::get_str_opt(row, "log_key"),
                    row.get::<_, i64>("my_sequence").unwrap_or(0).cast_unsigned(),
                    row.get::<_, i64>("notification_level").unwrap_or(0),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load member pseudonym keys for the known_members cache
        let mut member_stmt = conn
            .prepare(
                "SELECT community_id, pseudonym_key FROM community_members WHERE owner_key = ?1",
            )?;
        let member_keys = member_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "community_id"),
                    db::get_str(row, "pseudonym_key"),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut event_rsvp_stmt = conn.prepare(
            "SELECT community_id, event_id, status FROM community_event_rsvps WHERE owner_key = ?1",
        )?;
        let event_rsvp_rows = event_rsvp_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "community_id"),
                    db::get_str(row, "event_id"),
                    db::get_str(row, "status"),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok((communities, channels, role_rows, category_rows, member_keys, event_rsvp_rows))
    })
    .await?;

    let mut communities = state.communities.write();
    for (
        community_id,
        name,
        description,
        my_role,
        my_role_ids_json,
        dht_owner_keypair,
        my_pseudonym_key,
        mek_generation,
        db_member_registry_key,
        db_subkey_index,
        onboarding_complete,
    ) in &community_rows
    {
        let mut channel_log_keys = std::collections::HashMap::new();
        let mut channel_sequences = std::collections::HashMap::new();
        let channels: Vec<ChannelInfo> = channel_rows
            .iter()
            .filter(|(_, cid, _, _, _, _, _, _, _, _, _, _, _)| cid == community_id)
            .map(
                |(
                    id,
                    _,
                    ch_name,
                    ch_type,
                    cat_id,
                    topic,
                    slowmode,
                    nsfw,
                    msg_key,
                    mek_gen,
                    log_key,
                    my_seq,
                    notification_level,
                )| {
                    if let Some(ref lk) = log_key {
                        channel_log_keys.insert(id.clone(), lk.clone());
                    }
                    if *my_seq > 0 {
                        channel_sequences.insert(id.clone(), *my_seq);
                    }
                    ChannelInfo {
                        id: id.clone(),
                        name: ch_name.clone(),
                        channel_type: ch_type.clone(),
                        unread_count: 0,
                        category_id: cat_id.clone(),
                        topic: topic.clone(),
                        slowmode_seconds: *slowmode,
                        nsfw: *nsfw,
                        message_record_key: msg_key.clone(),
                        mek_generation: *mek_gen,
                        notification_level: match notification_level {
                            1 => "mentions".to_string(),
                            2 => "nothing".to_string(),
                            _ => "all".to_string(),
                        },
                    }
                },
            )
            .collect();

        let my_role_ids: Vec<u32> =
            serde_json::from_str(my_role_ids_json).unwrap_or_else(|_| vec![0, 1]);

        let roles: Vec<RoleDefinition> = role_rows
            .iter()
            .filter(|(cid, ..)| cid == community_id)
            .map(
                |(_, role_id, rname, color, permissions, position, hoist, mentionable, self_assignable)| {
                    RoleDefinition {
                        id: *role_id,
                        name: rname.clone(),
                        color: *color,
                        permissions: *permissions,
                        position: *position,
                        hoist: *hoist,
                        mentionable: *mentionable,
                        self_assignable: *self_assignable,
                    }
                },
            )
            .collect();

        let governance_key = Some(community_id.clone());
        let my_event_rsvps = event_rsvp_rows
            .iter()
            .filter(|(cid, _, _)| cid == community_id)
            .map(|(_, event_id, status)| (event_id.clone(), status.clone()))
            .collect();

        let mut community = CommunityState {
            id: community_id.clone(),
            name: name.clone(),
            description: description.clone(),
            channels,
            categories: category_rows
                .iter()
                .filter(|(cid, _, _, _)| cid == community_id)
                .map(|(_, id, name, sort_order)| CategoryInfo {
                    id: id.clone(),
                    name: name.clone(),
                    sort_order: *sort_order,
                })
                .collect(),
            my_role_ids,
            roles,
            my_role: Some(my_role.clone()),
            dht_owner_keypair: dht_owner_keypair.clone(),
            my_pseudonym_key: my_pseudonym_key.clone(),
            mek_generation: *mek_generation,
            member_registry_key: db_member_registry_key.clone(),
            my_subkey_index: *db_subkey_index,
            gossip: Some(crate::state::GossipOverlay::default()),
            slot_keypair: None,
            channel_log_keys,
            channel_sequences,
            pending_syncs: std::collections::HashMap::new(),
            watched_records: std::collections::HashSet::new(),
            record_sequences: std::collections::HashMap::new(),
            peer_sequences: std::collections::HashMap::new(),
            registry_owner_keypair: None,
            slot_seed: None,
            member_roles: std::collections::HashMap::new(),
            known_members: member_key_rows
                .iter()
                .filter(|(cid, _)| cid == community_id)
                .map(|(_, pk)| pk.clone())
                .collect(),
            presence_poll_shutdown_tx: None,
            dht_keepalive_shutdown_tx: None,
            open_community_records: crate::state::CommunityRecords::default(),
            my_event_rsvps,
            event_rsvps_by_event: std::collections::HashMap::new(),
            onboarding_complete: *onboarding_complete,
            governance_key,
            governance_state: None, // rebuilt from DHT during hydration
            lamport_counter: 0,
        };
        // Recalculate display role from role definitions (DB value may be stale),
        // but preserve "owner" — it's authoritative from SQLite and not derivable
        // from role definitions alone.
        if community.my_role.as_deref() != Some("owner") {
            community.my_role = Some(crate::state::display_role_name(
                &community.my_role_ids,
                &community.roles,
            ));
        }
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

    // Collect community IDs and whether we own them (have dht_owner_keypair)
    let community_info: Vec<(String, bool)> = {
        let communities = state.communities.read();
        communities
            .values()
            .map(|c| (c.id.clone(), c.dht_owner_keypair.is_some()))
            .collect()
    };

    let mut pseudonym_updates: Vec<(String, String)> = Vec::new();
    let mut mek_updates: Vec<(String, MediaEncryptionKey)> = Vec::new();
    let mut channel_mek_updates: Vec<(String, String, MediaEncryptionKey)> = Vec::new();
    let mut regenerated_community_ids: Vec<String> = Vec::new();

    for (community_id, is_owner) in &community_info {
        // Derive pseudonym
        let signing_key = derive_community_pseudonym(secret_key, community_id);
        let pseudonym_hex = hex::encode(signing_key.verifying_key().as_bytes());
        pseudonym_updates.push((community_id.clone(), pseudonym_hex));

        // Try to load MEK from Stronghold
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            if let Some(mek) = crate::keystore::load_mek(ks, community_id) {
                mek_updates.push((community_id.clone(), mek));
            } else if *is_owner {
                // Owned community with no MEK in Stronghold — regenerate.
                // This handles communities created before MEK persistence was added.
                tracing::warn!(
                    community = %community_id,
                    "MEK missing from Stronghold for owned community — regenerating"
                );
                let mek = MediaEncryptionKey::generate(1);
                crate::keystore::persist_mek(ks, community_id, &mek);
                mek_updates.push((community_id.clone(), mek));
                regenerated_community_ids.push(community_id.clone());
            } else {
                tracing::warn!(
                    community = %community_id,
                    "MEK missing from Stronghold for joined community — \
                     will be delivered when connecting to an online member"
                );
            }
        }
    }

    {
        let communities = state.communities.read();
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            for community in communities.values() {
                for channel in &community.channels {
                    let all = crate::keystore::load_all_meks(ks, &community.id, Some(&channel.id));
                    if let Some(mek) = all.into_iter().max_by_key(
                        rekindle_crypto::group::media_key::MediaEncryptionKey::generation,
                    ) {
                        channel_mek_updates.push((community.id.clone(), channel.id.clone(), mek));
                    }
                }
            }
        }
    }

    // Load slot/registry key material from Stronghold
    let mut slot_keypair_updates: Vec<(String, String)> = Vec::new();
    let mut slot_seed_updates: Vec<(String, String)> = Vec::new();
    let mut registry_keypair_updates: Vec<(String, String)> = Vec::new();
    {
        let keystore = keystore_handle.lock();
        if let Some(ref ks) = *keystore {
            for (community_id, _) in &community_info {
                if let Some(kp) = crate::keystore::load_slot_keypair(ks, community_id) {
                    slot_keypair_updates.push((community_id.clone(), kp));
                }
                if let Some(seed) = crate::keystore::load_slot_seed(ks, community_id) {
                    slot_seed_updates.push((community_id.clone(), seed));
                }
                if let Some(rkp) = crate::keystore::load_registry_keypair(ks, community_id) {
                    registry_keypair_updates.push((community_id.clone(), rkp));
                }
            }
        }
    }

    // Update communities with derived pseudonyms + keypairs
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

        for (community_id, kp) in slot_keypair_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.slot_keypair = Some(kp);
            }
        }
        for (community_id, seed) in slot_seed_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.slot_seed = Some(seed);
            }
        }
        for (community_id, rkp) in registry_keypair_updates {
            if let Some(c) = communities.get_mut(&community_id) {
                c.registry_owner_keypair = Some(rkp);
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

    {
        let mut channel_mek_cache = state.channel_mek_cache.lock();
        for (community_id, channel_id, mek) in channel_mek_updates {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                generation = mek.generation(),
                "restored channel MEK from Stronghold"
            );
            channel_mek_cache.insert((community_id, channel_id), mek);
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
    spawn_login_services(app, state, pool.clone(), prekey_bundle_bytes, dht_keys);
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

    // ── Phase 1-3: Open DHT records + hydrate + rebuild governance ──
    // These involve slow DHT network reads that can take 30-60+ seconds.
    // Run them in the background so login returns immediately with SQLite data.
    // The frontend can show channels/roles/members from SQLite right away;
    // background tasks will update state when DHT reads complete.
    {
        let bg_app = app.clone();
        let bg_state = Arc::clone(state);
        tokio::spawn(async move {
            open_community_dht_records(&bg_state).await;
            hydrate_community_state_from_dht(&bg_state).await;
            rebuild_governance_from_dht(&bg_state).await;
            tracing::info!("background DHT hydration complete — governance state rebuilt");

            // Emit GovernanceUpdated for each community so the frontend refreshes
            let community_ids: Vec<String> = bg_state.communities.read().keys().cloned().collect();
            for cid in &community_ids {
                let _ = bg_app.emit(
                    "community-event",
                    crate::channels::CommunityEvent::GovernanceUpdated {
                        community_id: cid.clone(),
                    },
                );
            }
            // Also emit MembersRefreshed so the frontend re-fetches members
            // even if the presence poll hasn't completed its first tick yet.
            for cid in &community_ids {
                let _ = bg_app.emit(
                    "community-event",
                    crate::channels::CommunityEvent::MembersRefreshed {
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
            services::community::start_presence_poll(Arc::clone(state), community_id.clone());
            services::community::start_dht_keepalive(Arc::clone(state), community_id);
        }
    }

    // ── Phase 5: Start local event reminder scheduler ──
    let reminder_handle = services::community::start_event_reminders(Arc::clone(state), pool.clone());

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

/// Open all community DHT records after login.
///
/// After app restart, Veilid closes all DHT records. Communities need the
/// governance, registry, and channel records open before background services
/// resume.
async fn open_community_dht_records(state: &SharedState) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        tracing::warn!("open_community_dht_records: no routing context, skipping");
        return;
    };
    struct CommunityRecordInfo {
        id: String,
        governance_key: String,
        registry_key: Option<String>,
        registry_writer: Option<String>,
    }
    let records: Vec<CommunityRecordInfo> = {
        let cs = state.communities.read();
        cs.values()
            .filter_map(|c| {
                c.governance_key
                    .as_ref()
                    .map(|governance_key| CommunityRecordInfo {
                        id: c.id.clone(),
                        governance_key: governance_key.clone(),
                        registry_key: c.member_registry_key.clone(),
                        registry_writer: c
                            .registry_owner_keypair
                            .clone()
                            .or_else(|| c.slot_keypair.clone()),
                    })
            })
            .collect()
    };

    for rec in &records {
        let community_id = &rec.id;
        let governance_key = &rec.governance_key;
        let registry_key = &rec.registry_key;
        let registry_writer = &rec.registry_writer;

        match governance_key.parse::<veilid_core::RecordKey>() {
            Ok(governance_typed_key) => {
                if let Err(e) = rc.open_dht_record(governance_typed_key, None).await {
                    tracing::debug!(
                        community = %community_id,
                        error = %e,
                        "failed to open governance record"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    community = %community_id,
                    error = %e,
                    "invalid governance key on login"
                );
                continue;
            }
        }

        if let Some(ref reg_key) = registry_key {
            let registry_opened = match reg_key.parse::<veilid_core::RecordKey>() {
                Ok(registry_typed_key) => {
                    if let Some(ref kp_str) = registry_writer {
                        if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
                            rc.open_dht_record(registry_typed_key, Some(kp))
                                .await
                                .is_ok()
                        } else {
                            false
                        }
                    } else {
                        rc.open_dht_record(registry_typed_key, None).await.is_ok()
                    }
                }
                Err(_) => false,
            };
            if !registry_opened {
                tracing::warn!(
                    community = %community_id,
                    "failed to open registry record on login"
                );
            }
        }

        let channel_log_keys: Vec<String> = {
            let cs = state.communities.read();
            cs.get(community_id.as_str())
                .map(|c| c.channel_log_keys.values().cloned().collect())
                .unwrap_or_default()
        };
        for key in &channel_log_keys {
            match key.parse::<veilid_core::RecordKey>() {
                Ok(channel_typed_key) => {
                    if let Err(e) = rc.open_dht_record(channel_typed_key, None).await {
                        tracing::debug!(
                            community = %community_id,
                            key,
                            error = %e,
                            "failed to open channel SMPL record on login"
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        community = %community_id,
                        key,
                        error = %e,
                        "invalid channel record key on login"
                    );
                }
            }
        }

        let mut all_keys = vec![governance_key.clone()];
        if let Some(ref rk) = registry_key {
            all_keys.push(rk.clone());
        }
        all_keys.extend(channel_log_keys.iter().cloned());
        state_helpers::track_open_records(state, &all_keys);

        {
            let mut cs = state.communities.write();
            if let Some(c) = cs.get_mut(community_id.as_str()) {
                c.open_community_records.governance_key = Some(governance_key.clone());
                c.open_community_records
                    .registry_key
                    .clone_from(registry_key);
                c.open_community_records
                    .registry_writer
                    .clone_from(registry_writer);
                c.open_community_records.channel_keys = channel_log_keys;
                c.open_community_records.records_open = true;
            }
        }

        if let Err(e) =
            crate::services::community::watch_community_records(state, community_id).await
        {
            tracing::debug!(
                community = %community_id,
                error = %e,
                "failed to watch community records after login open"
            );
        }
    }

    tracing::info!(
        count = records.len(),
        "opened community DHT records after login"
    );
}

/// Hydrate registry-linked community state from DHT.
async fn hydrate_community_state_from_dht(state: &SharedState) {
    use rekindle_protocol::dht::DHTManager;

    let Some(rc) = state_helpers::safe_routing_context(state) else {
        tracing::warn!("hydrate_community_state_from_dht: no routing context, skipping");
        return;
    };
    let mgr = DHTManager::new(rc);

    {
        use rekindle_protocol::dht::community::member_registry;

        // Collect registry info needed for async reads (can't hold lock across await)
        let registry_info: Vec<(String, String, Option<String>)> = {
            let communities = state.communities.read();
            communities
                .iter()
                .filter_map(|(cid, cs)| {
                    let rk = cs.member_registry_key.clone()?;
                    Some((cid.clone(), rk, cs.my_pseudonym_key.clone()))
                })
                .collect()
        };

        for (community_id, registry_key, my_pk) in &registry_info {
            let Some(pk) = my_pk else { continue };

            match member_registry::read_member_index(&mgr, registry_key).await {
                Ok(members) => {
                    if let Some(me) = members.iter().find(|m| m.pseudonym_key == *pk) {
                        let mut recovered_subkey = false;
                        let recovered_index = me.subkey_index;
                        let mut communities = state.communities.write();
                        if let Some(cs) = communities.get_mut(community_id.as_str()) {
                            // Recover my_subkey_index if missing
                            if cs.my_subkey_index.is_none() {
                                cs.my_subkey_index = Some(me.subkey_index);
                                recovered_subkey = true;
                                tracing::info!(
                                    community = %community_id,
                                    subkey_index = me.subkey_index,
                                    "recovered my_subkey_index from DHT registry"
                                );
                            }
                            // Update role_ids from DHT (authoritative) if richer
                            if !me.role_ids.is_empty() && me.role_ids.len() >= cs.my_role_ids.len()
                            {
                                cs.my_role_ids.clone_from(&me.role_ids);
                            }
                        }
                        drop(communities);

                        // Persist recovered my_subkey_index + role_ids to SQLite so they survive restarts
                        let role_ids_to_persist = {
                            let communities = state.communities.read();
                            communities
                                .get(community_id.as_str())
                                .map(|cs| cs.my_role_ids.clone())
                        };
                        if recovered_subkey || role_ids_to_persist.is_some() {
                            let app_handle = state.app_handle.read().clone();
                            if let Some(ref ah) = app_handle {
                                let pool: tauri::State<'_, crate::db::DbPool> = ah.state();
                                let ok =
                                    state_helpers::current_owner_key(state).unwrap_or_default();
                                let cid = community_id.clone();
                                let idx = recovered_index;
                                let roles_json = role_ids_to_persist
                                    .and_then(|r| serde_json::to_string(&r).ok());
                                crate::db_helpers::db_fire(
                                    pool.inner(),
                                    "persist hydrated subkey_index + role_ids",
                                    move |conn| {
                                        if recovered_subkey {
                                            conn.execute(
                                            "UPDATE communities SET my_subkey_index = ?1 WHERE owner_key = ?2 AND id = ?3",
                                            rusqlite::params![idx, &ok, &cid],
                                        )?;
                                        }
                                        if let Some(rj) = roles_json {
                                            conn.execute(
                                            "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                                            rusqlite::params![rj, &ok, &cid],
                                        )?;
                                        }
                                        Ok(())
                                    },
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        community = %community_id,
                        error = %e,
                        "failed to read member registry during hydration"
                    );
                }
            }
        }

        // Derive slot_keypair immediately if seed + index available (no 60s wait)
        for (community_id, _, _) in &registry_info {
            let should_derive = {
                let communities = state.communities.read();
                communities.get(community_id.as_str()).and_then(|cs| {
                    if cs.slot_keypair.is_none() {
                        cs.slot_seed
                            .as_ref()
                            .and_then(|seed| cs.my_subkey_index.map(|idx| (seed.clone(), idx)))
                    } else {
                        None
                    }
                })
            };
            if let Some((seed, idx)) = should_derive {
                services::community::try_derive_slot_keypair(state, community_id, &seed, idx);
            }
        }

        // Belt-and-suspenders: recover registry_owner_keypair from Stronghold
        // if login didn't load it (e.g. race condition, Stronghold unlock timing).
        let missing_registry_kp: Vec<String> = {
            let communities = state.communities.read();
            registry_info
                .iter()
                .filter(|(cid, _, _)| {
                    communities
                        .get(cid.as_str())
                        .is_some_and(|cs| cs.registry_owner_keypair.is_none())
                })
                .map(|(cid, _, _)| cid.clone())
                .collect()
        };
        if !missing_registry_kp.is_empty() {
            let app_handle = state.app_handle.read().clone();
            if let Some(ref ah) = app_handle {
                let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = ah.state();
                let ks_guard = ks_handle.lock();
                if let Some(ref ks) = *ks_guard {
                    let mut communities = state.communities.write();
                    for cid in &missing_registry_kp {
                        if let Some(rkp) = crate::keystore::load_registry_keypair(ks, cid) {
                            tracing::info!(community = %cid, "recovered registry_owner_keypair from Stronghold during hydrate");
                            if let Some(cs) = communities.get_mut(cid.as_str()) {
                                cs.registry_owner_keypair = Some(rkp);
                            }
                        }
                    }
                }
            }
        }
    }
    tracing::info!("hydrated community registry-linked state from DHT");
}

/// Rebuild governance state from SMPL governance records for all v2.0 communities.
///
/// For each community with a `governance_key`, opens the governance SMPL record,
/// reads all 255 subkeys, runs CRDT merge, and caches the result.
/// Communities without `governance_key` are skipped (v1.0 communities).
async fn rebuild_governance_from_dht(state: &SharedState) {
    let Some(rc) = state_helpers::safe_routing_context(state) else {
        tracing::warn!("rebuild_governance_from_dht: no routing context, skipping");
        return;
    };

    // Collect communities that have governance keys
    let communities: Vec<(String, String)> = {
        let cs = state.communities.read();
        cs.values()
            .filter_map(|c| {
                c.governance_key
                    .as_ref()
                    .map(|gk| (c.id.clone(), gk.clone()))
            })
            .collect()
    };

    for (community_id, gov_key_str) in &communities {
        let Ok(gov_key) = gov_key_str.parse::<veilid_core::RecordKey>() else {
            tracing::warn!(community = %community_id, "invalid governance key — skipping hydration");
            continue;
        };

        // Open governance record (may already be open from a previous session)
        if let Err(e) = rc.open_dht_record(gov_key.clone(), None).await {
            tracing::debug!(community = %community_id, error = %e, "failed to open governance record for hydration");
            continue;
        }

        // Use inspect to find which subkeys have been written (seq > 0),
        // then only read those. This avoids 255 sequential DHT reads on startup.
        let occupied_subkeys: Vec<u32> = match rc
            .inspect_dht_record(
                gov_key.clone(),
                Some(veilid_core::ValueSubkeyRangeSet::full()),
                veilid_core::DHTReportScope::UpdateGet,
            )
            .await
        {
            Ok(report) => {
                // Use network_seqs — reflects what exists on the DHT network,
                // not just our local cache (which may be empty after restart)
                report
                    .network_seqs()
                    .iter()
                    .enumerate()
                    .filter(|(_, &seq)| seq != veilid_core::ValueSeqNum::default())
                    .map(|(i, _)| u32::try_from(i).unwrap_or(0))
                    .collect()
            }
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "governance inspect failed — falling back to full scan");
                (0..255u32).collect()
            }
        };

        // Read occupied subkeys concurrently (bounded to 10 at a time)
        let mut all_entries: Vec<(
            rekindle_types::id::PseudonymKey,
            Vec<rekindle_types::governance::GovernanceEntry>,
        )> = Vec::new();
        {
            use futures::stream::{FuturesUnordered, StreamExt};
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(10));
            let mut futs = FuturesUnordered::new();
            for subkey in occupied_subkeys {
                let sem = sem.clone();
                let rc = rc.clone();
                let gk = gov_key.clone();
                futs.push(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let result = rc.get_dht_value(gk, subkey, false).await;
                    (subkey, result)
                });
            }
            while let Some((_subkey, result)) = futs.next().await {
                if let Ok(Some(val)) = result {
                    if !val.data().is_empty() {
                        if let Ok(payload) = serde_json::from_slice::<
                            rekindle_types::governance::GovernanceSubkeyPayload,
                        >(val.data())
                        {
                            all_entries.push((payload.author_pseudonym, payload.entries));
                        }
                    }
                }
            }
        }

        if all_entries.is_empty() {
            tracing::debug!(community = %community_id, "governance record empty — no entries to merge");
            continue;
        }

        let previous_bans = state_helpers::governance_state(state, community_id)
            .map(|gov| gov.bans)
            .unwrap_or_default();

        // CRDT merge
        let gov_state = rekindle_governance::merge::merge(&all_entries);
        let new_bans: Vec<String> = gov_state
            .bans
            .iter()
            .filter(|pseudo| !previous_bans.contains(*pseudo))
            .map(|pseudo| hex::encode(pseudo.0))
            .collect();

        // Restore lamport_counter from highest lamport in governance entries
        // so new entries don't collide with existing ones
        let max_lamport = all_entries
            .iter()
            .flat_map(|(_, entries)| {
                entries
                    .iter()
                    .map(rekindle_types::governance::GovernanceEntry::lamport)
            })
            .max()
            .unwrap_or(0);

        {
            let mut communities = state.communities.write();
            if let Some(cs) = communities.get_mut(community_id.as_str()) {
                cs.lamport_counter = cs.lamport_counter.max(max_lamport);
            }
        }

        state_helpers::set_governance_state(state, community_id, gov_state);

        let app_handle = { state.app_handle.read().clone() };
        if let Some(app_handle) = app_handle {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            if let Err(e) = state_helpers::persist_governance_snapshot_to_sqlite(
                state,
                pool.inner(),
                community_id,
                max_lamport,
            )
            .await
            {
                tracing::warn!(
                    community = %community_id,
                    error = %e,
                    "failed to persist rebuilt governance snapshot"
                );
            }
        }

        tracing::info!(community = %community_id, max_lamport, "rebuilt governance state from DHT");

        if let Some(app_handle) = state_helpers::app_handle(state) {
            for banned_pseudonym in new_bans {
                let state = state.clone();
                let app_handle = app_handle.clone();
                let community_id = community_id.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = crate::services::community::rotate_text_mek_for_departure(
                        &app_handle,
                        &state,
                        &community_id,
                        &banned_pseudonym,
                    )
                    .await
                    {
                        tracing::debug!(community = %community_id, member = %banned_pseudonym, error = %error, "text MEK rotation skipped after governance ban sync");
                    }
                });
            }
        }
    }
}

/// Public wrapper for `rebuild_governance_from_dht` — called from the network
/// reconnection handler when the routing context wasn't available at login time.
pub async fn rebuild_governance_from_dht_public(state: &SharedState) {
    rebuild_governance_from_dht(state).await;
}

/// Public wrapper for `open_community_dht_records` — called from the network
/// reconnection handler when the routing context wasn't available at login time.
pub async fn open_community_dht_records_public(state: &SharedState) {
    open_community_dht_records(state).await;
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
        let api = state_helpers::veilid_api(state)?;

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

    // Status is published by publish_profile_to_dht() (subkey 2) alongside the fresh
    // route blob — no early publish here to avoid "value is not writable" (record not
    // yet opened with keypair) and stale-route-blob races.

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
    // Also reset `needs_initial_sync` so the PresenceUpdate gets re-broadcast
    // with the real route_blob (the Phase 5 first tick likely broadcast with None).
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
    // Uses Edwards->Montgomery birational map on the PUBLIC key (NOT from_secret_bytes).
    let friend_ed_bytes = hex::decode(friend_public_key)
        .map_err(|e| format!("invalid friend public key hex: {e}"))?;
    let friend_ed_bytes: [u8; 32] = friend_ed_bytes
        .try_into()
        .map_err(|_| "friend public key must be 32 bytes".to_string())?;
    let friend_x25519_public = rekindle_crypto::Identity::peer_ed25519_to_x25519(&friend_ed_bytes)
        .map_err(|e| format!("failed to convert friend key to X25519: {e}"))?;

    let encryption_key = rekindle_crypto::DhtRecordKey::derive_conversation_key(
        &my_x25519_secret,
        &friend_x25519_public,
    );

    let id = state_helpers::current_identity(state)?;
    let (display_name, status_message, owner_key) =
        (id.display_name, id.status_message, id.public_key);

    let routing_context = state_helpers::require_safe_routing_context(state)?;

    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();

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

    state_helpers::track_open_records(state, &record.all_record_keys());

    // Persist to SQLite
    let ok = owner_key;
    let fpk = friend_public_key.to_string();
    let ck = conversation_key.clone();
    db_call(pool, move |conn| {
        crate::friend_repo::update_local_conversation_key(conn, &ok, &fpk, &ck)
    })
    .await?;

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
fn initialize_signal_manager(state: &SharedState, secret_key: &[u8; 32]) -> Option<Vec<u8>> {
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
    let registration_id =
        u32::from_le_bytes([pub_bytes[0], pub_bytes[1], pub_bytes[2], pub_bytes[3]]);

    let identity_store =
        MemoryIdentityStore::new(identity_private, identity_public, registration_id);
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
