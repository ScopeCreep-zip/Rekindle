//! E2E test server — HTTP bridge to the real Rust backend.
//!
//! Exposes Tauri IPC commands over HTTP so Playwright can test the real
//! `SolidJS` frontend against real `SQLite` + Stronghold + Ed25519 backend.
//!
//! ```bash
//! cargo run -p rekindle --bin e2e-server --features e2e-server
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

use rekindle_lib::commands::auth::{create_identity_core, login_core, LoginResult, IdentitySummary};
use rekindle_lib::db::{self, DbPool};
use rekindle_lib::keystore::{self, KeystoreHandle, StrongholdKeystore};
use rekindle_lib::state::{AppState, SharedState, UserStatus};

/// Shared server state passed to every axum handler.
struct ServerState {
    state: SharedState,
    pool: DbPool,
    keystore_handle: KeystoreHandle,
    config_dir: PathBuf,
}

type SharedServer = Arc<ServerState>;

#[derive(Deserialize)]
struct InvokeRequest {
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config_dir = std::env::temp_dir().join(format!("rekindle-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&config_dir).expect("failed to create temp config dir");

    tracing::info!(dir = %config_dir.display(), "E2E server starting");

    let pool = db::create_pool(":memory:").expect("in-memory SQLite").pool;
    let shared_state: SharedState = Arc::new(AppState::default());
    let keystore_handle = keystore::new_handle();

    let server = Arc::new(ServerState {
        state: shared_state,
        pool,
        keystore_handle,
        config_dir: config_dir.clone(),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/invoke", post(handle_invoke))
        .route("/reset", post(handle_reset))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .layer(cors)
        .with_state(server);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3001")
        .await
        .expect("failed to bind to port 3001");

    tracing::info!("E2E server listening on http://127.0.0.1:3001");
    axum::serve(listener, app).await.expect("server error");
}

/// Dispatch an IPC command to the real backend core functions.
async fn handle_invoke(
    AxumState(server): AxumState<SharedServer>,
    Json(req): Json<InvokeRequest>,
) -> impl IntoResponse {
    tracing::debug!(cmd = %req.cmd, "invoke");

    match dispatch(&server, &req.cmd, &req.args).await {
        Ok(result) => (StatusCode::OK, Json(json!({ "result": result }))),
        Err(e) => {
            tracing::warn!(cmd = %req.cmd, error = %e, "command failed");
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e })),
            )
        }
    }
}

/// Reset all server state for test isolation.
async fn handle_reset(AxumState(server): AxumState<SharedServer>) -> impl IntoResponse {
    tracing::info!("resetting server state");

    // Clear in-memory state
    *server.state.identity.write() = None;
    server.state.friends.write().clear();
    server.state.communities.write().clear();
    *server.keystore_handle.lock() = None;

    // Recreate SQLite database (drop all tables and re-run schema)
    {
        let conn = server.pool.lock().expect("db lock");
        conn.execute_batch(
            "DELETE FROM pending_messages; \
             DELETE FROM prekeys; \
             DELETE FROM signal_sessions; \
             DELETE FROM trusted_identities; \
             DELETE FROM messages; \
             DELETE FROM community_members; \
             DELETE FROM channels; \
             DELETE FROM communities; \
             DELETE FROM friends; \
             DELETE FROM friend_groups; \
             DELETE FROM identity;",
        )
        .expect("failed to clear database");
    }

    // Delete Stronghold snapshot files
    if let Ok(entries) = std::fs::read_dir(&server.config_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "stronghold")
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    StatusCode::OK
}

/// Route commands to the appropriate core function.
async fn dispatch(server: &ServerState, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        // ── Auth ─────────────────────────────────────────────────────
        "create_identity" => {
            let passphrase = arg_str(args, "passphrase")?;
            let display_name = arg_str_opt(args, "displayName");
            let (result, _) = create_identity_core(
                &server.config_dir,
                &passphrase,
                display_name,
                &server.state,
                &server.pool,
                &server.keystore_handle,
            )
            .await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "login" => {
            let public_key = arg_str(args, "publicKey")?;
            let passphrase = arg_str(args, "passphrase")?;
            let (result, _, _) = login_core(
                &server.config_dir,
                &public_key,
                &passphrase,
                &server.state,
                &server.pool,
                &server.keystore_handle,
            )
            .await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "list_identities" => {
            let pool = server.pool.clone();
            let summaries = tokio::task::spawn_blocking(move || {
                let conn = pool.lock().map_err(|e| e.to_string())?;
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
                            public_key: row.get::<_, String>(0)?,
                            display_name: row.get::<_, String>(1).unwrap_or_default(),
                            created_at: row.get::<_, i64>(2)?,
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
            .map_err(|e| e.to_string())??;
            Ok(serde_json::to_value(summaries).unwrap())
        }
        "delete_identity" => {
            let public_key = arg_str(args, "publicKey")?;
            let passphrase = arg_str(args, "passphrase")?;

            // Verify passphrase
            StrongholdKeystore::initialize_for_identity(&server.config_dir, &public_key, &passphrase)
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("snapshot") || msg.contains("decrypt") {
                        "Wrong passphrase".to_string()
                    } else {
                        msg
                    }
                })?;

            // If deleting active identity, clear state
            let is_active = server.state.identity.read()
                .as_ref()
                .is_some_and(|id| id.public_key == public_key);
            if is_active {
                *server.state.identity.write() = None;
                server.state.friends.write().clear();
                server.state.communities.write().clear();
                *server.keystore_handle.lock() = None;
            }

            // Delete from DB
            let pool = server.pool.clone();
            let pk = public_key.clone();
            tokio::task::spawn_blocking(move || {
                let conn = pool.lock().map_err(|e| e.to_string())?;
                conn.execute("DELETE FROM identity WHERE public_key = ?1", rusqlite::params![pk])
                    .map_err(|e| e.to_string())?;
                Ok::<(), String>(())
            })
            .await
            .map_err(|e| e.to_string())??;

            // Delete Stronghold file
            let _ = StrongholdKeystore::delete_snapshot(&server.config_dir, &public_key);

            Ok(Value::Null)
        }
        "get_identity" => {
            let identity = server.state.identity.read();
            Ok(match identity.as_ref() {
                Some(id) => serde_json::to_value(LoginResult {
                    public_key: id.public_key.clone(),
                    display_name: id.display_name.clone(),
                })
                .unwrap(),
                None => Value::Null,
            })
        }
        "logout" => {
            *server.state.identity.write() = None;
            server.state.friends.write().clear();
            server.state.communities.write().clear();
            *server.keystore_handle.lock() = None;
            Ok(Value::Null)
        }

        // ── Friends ──────────────────────────────────────────────────
        "get_friends" => {
            let friends = server.state.friends.read();
            let list: Vec<Value> = friends
                .values()
                .map(|f| {
                    json!({
                        "publicKey": f.public_key,
                        "displayName": f.display_name,
                        "nickname": f.nickname,
                        "status": format!("{}", match f.status {
                            UserStatus::Online => "online",
                            UserStatus::Away => "away",
                            UserStatus::Busy => "busy",
                            UserStatus::Offline => "offline",
                        }),
                        "statusMessage": f.status_message,
                        "gameInfo": Value::Null,
                        "group": f.group,
                        "unreadCount": f.unread_count,
                        "lastSeenAt": f.last_seen_at,
                    })
                })
                .collect();
            Ok(Value::Array(list))
        }

        // ── Communities ──────────────────────────────────────────────
        "get_community_details" | "get_communities" => Ok(json!([])),

        // ── Settings ─────────────────────────────────────────────────
        "get_preferences" => Ok(json!({
            "notificationsEnabled": true,
            "notificationSound": true,
            "startMinimized": false,
            "autoStart": false,
            "gameDetectionEnabled": true,
            "gameScanIntervalSecs": 30
        })),

        // ── Game / Network ───────────────────────────────────────────
        "get_network_status" => Ok(json!({
            "isAttached": false,
            "publicInternetReady": false,
            "hasRoute": false,
            "profileDhtKey": Value::Null,
            "friendListDhtKey": Value::Null,
        })),

        // ── No-op commands (window management, status, etc.) ─────────
        "get_game_status" | "show_buddy_list" | "open_chat_window"
        | "open_settings_window" | "open_community_window" | "open_profile_window"
        | "set_status" | "set_nickname" | "set_avatar" | "set_status_message"
        | "set_mute" | "set_deafen" | "check_for_updates" => Ok(Value::Null),

        _ => Err(format!("unknown command: {cmd}")),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(String::from)
}
