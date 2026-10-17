//! E2E test server — HTTP bridge to the real Rust backend.
//!
//! Exposes Tauri IPC commands over HTTP so Playwright can test the real
//! `SolidJS` frontend against the real identity store (vault, SQLCipher
//! database, registry) and Ed25519, in a temp data root.
//!
//! ```bash
//! cargo run -p rekindle --bin e2e-server --features e2e-server
//! ```

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

use rekindle_db::paths::DataRoot;
use rekindle_lib::commands::auth::{
    create_identity_core, delete_identity_core, list_identities_inner, lock_identity, login_core,
    verify_identity_passphrase_core, LoginResult,
};
use rekindle_lib::state::{AppState, SharedState, UserStatus};

/// Shared server state passed to every axum handler.
struct ServerState {
    state: SharedState,
    root: DataRoot,
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
    rekindle_utils::log_scrub::install_panic_hook();
    tracing_subscriber::fmt()
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(
            std::io::stdout,
        ))
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let dir = std::env::temp_dir().join(format!("rekindle-e2e-{}", std::process::id()));
    let data = dir.join("data");
    let root = DataRoot {
        config: dir.join("config"),
        logs: dir.join("logs"),
        state: data.join("state"),
        data,
    };
    // The host claims the temp root (its lock and layout check) and runs
    // with no node: the E2E suite drives the identity lifecycle only.
    let claimed = rekindle_node::host::claim(root.clone()).expect("claim of the temp data root");

    tracing::info!(dir = %dir.display(), "E2E server starting");

    let state: SharedState = Arc::new(AppState::default());
    let host = rekindle_node::host::Host::start(rekindle_node::host::HostConfig {
        root: claimed,
        stores: state.host_stores(),
        transport: rekindle_node::host::TransportSource::Offline,
        serve_bus: false,
    })
    .await
    .expect("the E2E host did not start");
    state.host.set(host);
    let server = Arc::new(ServerState { state, root });

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
            (StatusCode::BAD_REQUEST, Json(json!({ "error": e })))
        }
    }
}

/// Reset all server state for test isolation.
async fn handle_reset(AxumState(server): AxumState<SharedServer>) -> impl IntoResponse {
    tracing::info!("resetting server state");

    lock_identity(&server.state).await;
    server.state.friends.write().clear();
    server.state.communities.write().clear();

    // Every identity's files and the registry.
    if rekindle_db::reset::run(&[rekindle_db::reset::ResetDir {
        path: server.root.state.clone(),
        keep: &["layout-version"],
    }])
    .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR;
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
            let (result, _) =
                create_identity_core(&server.root, &passphrase, display_name, &server.state, None)
                    .await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "login" => {
            let public_key = arg_str(args, "publicKey")?;
            let passphrase = arg_str(args, "passphrase")?;
            let (result, _, _) =
                login_core(&server.root, &public_key, &passphrase, &server.state, None).await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "list_identities" => {
            let summaries = list_identities_inner(&server.root)?;
            Ok(serde_json::to_value(summaries).unwrap())
        }
        "delete_identity" => {
            let public_key = arg_str(args, "publicKey")?;
            let passphrase = arg_str(args, "passphrase")?;

            verify_identity_passphrase_core(&server.root, &server.state, &public_key, &passphrase)
                .await?;
            let is_active = server
                .state
                .identity
                .read()
                .as_ref()
                .is_some_and(|id| id.public_key == public_key);
            if is_active {
                lock_identity(&server.state).await;
                server.state.friends.write().clear();
                server.state.communities.write().clear();
            }
            delete_identity_core(&server.root, &server.state, &public_key, &passphrase).await?;
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
            lock_identity(&server.state).await;
            server.state.friends.write().clear();
            server.state.communities.write().clear();
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
                            UserStatus::Offline | UserStatus::Invisible => "offline",
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
            "mediaRoute": "idle",
            "profileDhtKey": Value::Null,
            "friendListDhtKey": Value::Null,
        })),

        // ── Lifecycle ────────────────────────────────────────────────
        // The frontend lifecycle store seeds from this on mount and gates
        // the login button on `canUnlock()` (state == "locked"). The test
        // double models readiness so the button is enabled in the harness;
        // keeps "one lifecycle" — no VITE_E2E branch in product code.
        "lifecycle_current" => Ok(json!("locked")),

        // ── No-op commands (window management, status, etc.) ─────────
        "get_game_status"
        | "show_buddy_list"
        | "open_chat_window"
        | "open_settings_window"
        | "open_community_window"
        | "open_profile_window"
        | "set_status"
        | "set_nickname"
        | "set_avatar"
        | "set_status_message"
        | "set_mute"
        | "set_deafen"
        | "check_for_updates" => Ok(Value::Null),

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
