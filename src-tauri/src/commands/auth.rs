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

/// Get the current identity's public key from AppState.
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
    start_background_services(app, state.inner(), pool.inner(), &secret_bytes, None, None, None, None, None, None);

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
                     account_dht_key, account_owner_keypair \
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
        app,
        state.inner(),
        pool.inner(),
        &secret_key,
        dht_cols.existing_dht_key,
        dht_cols.existing_friend_list_key,
        dht_cols.dht_owner_keypair,
        dht_cols.friend_list_owner_keypair,
        dht_cols.account_dht_key,
        dht_cols.account_owner_keypair,
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
/// Returns summaries of every identity in SQLite, ordered by creation date.
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
                 g.name AS group_name \
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
async fn load_communities_from_db(
    pool: &DbPool,
    state: &SharedState,
    owner_key: &str,
) -> Result<(), String> {
    let db = pool.clone();
    let ok = owner_key.to_string();
    let (community_rows, channel_rows) = tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;

        let mut comm_stmt = conn
            .prepare("SELECT id, name, description, my_role, dht_record_key FROM communities WHERE owner_key = ?1")
            .map_err(|e| e.to_string())?;
        let communities = comm_stmt
            .query_map(rusqlite::params![ok], |row| {
                Ok((
                    db::get_str(row, "id"),
                    db::get_str(row, "name"),
                    db::get_str_opt(row, "description"),
                    db::get_str(row, "my_role"),
                    db::get_str_opt(row, "dht_record_key"),
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

        Ok::<(Vec<_>, Vec<_>), String>((communities, channels))
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut communities = state.communities.write();
    for (community_id, name, description, my_role, dht_record_key) in &community_rows {
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

        let community = CommunityState {
            id: community_id.clone(),
            name: name.clone(),
            description: description.clone(),
            channels,
            my_role: Some(my_role.clone()),
            dht_record_key: dht_record_key.clone(),
        };
        communities.insert(community_id.clone(), community);
    }
    Ok(())
}

/// Initialize Signal encryption and spawn all background services (non-blocking).
///
/// Returns immediately after setting up in-memory state. Uses the already-running
/// Veilid node (started at app launch) for DHT publishing, sync, and messaging.
/// Game detection and sync services are spawned as background tasks so login
/// returns near-instantly to the frontend.
fn start_background_services(
    app: tauri::AppHandle,
    state: &SharedState,
    pool: &DbPool,
    secret_key: &[u8; 32],
    existing_dht_key: Option<String>,
    existing_friend_list_key: Option<String>,
    dht_owner_keypair: Option<String>,
    friend_list_owner_keypair: Option<String>,
    account_dht_key: Option<String>,
    account_owner_keypair: Option<String>,
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

    // The Veilid node is already running (started at app startup).
    // Just spawn sync + DHT publish as background tasks.
    let services_state = Arc::clone(state);
    let services_pool = pool.clone();
    let services_handle = tauri::async_runtime::spawn(async move {
        spawn_login_services(
            app,
            services_state,
            services_pool,
            prekey_bundle_bytes,
            existing_dht_key,
            existing_friend_list_key,
            dht_owner_keypair,
            friend_list_owner_keypair,
            account_dht_key,
            account_owner_keypair,
        )
        .await;
    });

    // Store handles so logout can abort them
    {
        let mut handles = state.background_handles.lock();
        handles.push(game_handle);
        handles.push(services_handle);
    }
}

/// Background task: start sync service and DHT publish using the existing node.
///
/// The Veilid node and dispatch loop are already running (started at app startup).
/// This function only spawns user-specific services: sync and DHT publish.
async fn spawn_login_services(
    app: tauri::AppHandle,
    state: SharedState,
    pool: DbPool,
    prekey_bundle_bytes: Option<Vec<u8>>,
    existing_dht_key: Option<String>,
    existing_friend_list_key: Option<String>,
    dht_owner_keypair: Option<String>,
    friend_list_owner_keypair: Option<String>,
    account_dht_key: Option<String>,
    account_owner_keypair: Option<String>,
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
            if let Some(ref dht_key) = existing_dht_key {
                nh.profile_dht_key = Some(dht_key.clone());
            }
            if let Some(ref fl_key) = existing_friend_list_key {
                nh.friend_list_dht_key = Some(fl_key.clone());
            }
        }
    }

    // Create sync service shutdown channel
    let (sync_shutdown_tx, sync_shutdown_rx) = mpsc::channel::<()>(1);

    // Spawn the periodic sync service
    let sync_state = Arc::clone(&state);
    let sync_pool = pool.clone();
    let sync_handle = tauri::async_runtime::spawn(async move {
        services::sync_service::start_sync_loop(sync_state, sync_pool, sync_shutdown_rx).await;
    });

    *state.sync_shutdown_tx.write() = Some(sync_shutdown_tx);

    // Spawn DHT publish as a background task
    let dht_handle = tauri::async_runtime::spawn(spawn_dht_publish(
        app,
        state.clone(),
        pool,
        prekey_bundle_bytes,
        existing_dht_key,
        existing_friend_list_key,
        dht_owner_keypair,
        friend_list_owner_keypair,
        account_dht_key,
        account_owner_keypair,
    ));

    // Store sub-task handles so they can be aborted on logout
    {
        let mut handles = state.background_handles.lock();
        handles.push(sync_handle);
        handles.push(dht_handle);
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
    existing_dht_key: Option<String>,
    existing_friend_list_key: Option<String>,
    dht_owner_keypair: Option<String>,
    friend_list_owner_keypair: Option<String>,
    account_dht_key: Option<String>,
    account_owner_keypair: Option<String>,
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

    tracing::info!("public internet ready — publishing profile to DHT");

    if let Err(e) = publish_profile_to_dht(
        &state,
        &pool,
        prekey_bundle_bytes,
        existing_dht_key,
        dht_owner_keypair,
    )
    .await
    {
        tracing::warn!(error = %e, "DHT profile publish failed — will retry on next sync");
    }

    if let Err(e) =
        publish_friend_list_to_dht(&state, &pool, existing_friend_list_key, friend_list_owner_keypair)
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
        account_dht_key,
        account_owner_keypair,
    )
    .await
    {
        tracing::warn!(error = %e, "DHT account publish failed — will retry on next sync");
    }
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
            nh.profile_owner_keypair = effective_keypair.clone();
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

/// Create (or reopen) the private Account DHT record after login.
///
/// The account record is encrypted with a key derived from the identity's Ed25519 secret.
/// It holds pointers to contact list, chat list, and invitation list DHTShortArrays.
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
                    let all_keys = record.all_record_keys();
                    // Store account key on NodeHandle and track records
                    if let Some(ref mut nh) = *state.node.write() {
                        nh.account_dht_key = Some(record.record_key());
                    }
                    {
                        let mut dht_mgr = state.dht_manager.write();
                        if let Some(ref mut mgr) = *dht_mgr {
                            for k in all_keys {
                                mgr.track_open_record(k);
                            }
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        key = %existing_key, error = %e,
                        "failed to open existing account record — creating new one"
                    );
                    let enc_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
                    let (record, kp) = rekindle_protocol::dht::account::AccountRecord::create(
                        &routing_context,
                        enc_key,
                        &display_name,
                        &status_message,
                    )
                    .await
                    .map_err(|e| format!("create account record: {e}"))?;
                    (record.record_key(), Some(kp))
                }
            }
        } else {
            tracing::warn!("no account owner keypair — creating new account record");
            let enc_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
            let (record, kp) = rekindle_protocol::dht::account::AccountRecord::create(
                &routing_context,
                enc_key,
                &display_name,
                &status_message,
            )
            .await
            .map_err(|e| format!("create account record: {e}"))?;
            (record.record_key(), Some(kp))
        }
    } else {
        let (record, kp) = rekindle_protocol::dht::account::AccountRecord::create(
            &routing_context,
            encryption_key,
            &display_name,
            &status_message,
        )
        .await
        .map_err(|e| format!("create account record: {e}"))?;
        (record.record_key(), Some(kp))
    };

    // Store account key on NodeHandle and track the record
    if let Some(ref mut nh) = *state.node.write() {
        nh.account_dht_key = Some(account_key.clone());
    }
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.track_open_record(account_key.clone());
        }
    }

    // Persist to SQLite
    let keypair_str = new_keypair.map(|kp| kp.to_string());
    let db = pool.clone();
    let pk = public_key;
    let ak = account_key.clone();
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
    .map_err(|e| e.to_string())??;

    tracing::info!(
        account_key = %account_key,
        "published account record to DHT"
    );
    Ok(())
}

/// Create a conversation record for a specific friend.
///
/// Called when establishing a new contact. Creates a ConversationRecord,
/// and persists the keys to SQLite.
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

    // Parse friend's Ed25519 public key and derive X25519 public key
    let friend_ed_bytes = hex::decode(friend_public_key)
        .map_err(|e| format!("invalid friend public key hex: {e}"))?;
    let friend_ed_bytes: [u8; 32] = friend_ed_bytes
        .try_into()
        .map_err(|_| "friend public key must be 32 bytes")?;
    let friend_identity = rekindle_crypto::Identity::from_secret_bytes(&friend_ed_bytes);
    let friend_x25519_public = friend_identity.to_x25519_public();

    let encryption_key =
        rekindle_crypto::DhtRecordKey::derive_conversation_key(&my_x25519_secret, &friend_x25519_public);

    let (display_name, status_message, owner_key) = {
        let id = state.identity.read();
        let id = id.as_ref().ok_or("not logged in")?;
        (id.public_key.clone(), id.status_message.clone(), id.public_key.clone())
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
