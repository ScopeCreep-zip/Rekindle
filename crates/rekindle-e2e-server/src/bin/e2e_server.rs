//! E2E test server — HTTP bridge to the real Rust backend.
//!
//! Exposes Tauri IPC commands over HTTP so Playwright can test the real
//! `SolidJS` frontend against the real identity store (vault, SQLCipher
//! database, registry) and Ed25519, in a temp data root. Host calls go
//! through the same `host_call::call` the window uses, against an Offline
//! Host with the GUI's `ShellSession` hooks (plan E1.1).
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
use rekindle_ipc::protocol::IpcRequest;
use rekindle_lib::host_call;
use rekindle_lib::state::{AppState, SharedState};
use rekindle_types::dto::{CommandError, Done};

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
    // with no node, with the GUI's shell hooks: the E2E suite drives the
    // identity lifecycle through the same Host calls the window makes.
    let claimed = rekindle_node::host::claim(root.clone()).expect("claim of the temp data root");

    tracing::info!(dir = %dir.display(), "E2E server starting");

    let state: SharedState = Arc::new(AppState::default());
    let host = rekindle_node::host::Host::start(rekindle_node::host::HostConfig {
        root: claimed,
        stores: state.host_stores(),
        transport: rekindle_node::host::TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
        shell: Some(rekindle_lib::ShellSession::new(Arc::clone(&state), None) as _),
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

    // A Host call answers `{ result, undo }` or `{ error: CommandError }`,
    // the shapes the Tauri path resolves and rejects with.
    match dispatch(&server, &req.cmd, &req.args).await {
        Ok(Answer::Host(done)) => (
            StatusCode::OK,
            Json(json!({ "result": done.value, "undo": done.undo })),
        ),
        Ok(Answer::Local(value)) => (StatusCode::OK, Json(json!({ "result": value }))),
        Err(e) => {
            tracing::warn!(cmd = %req.cmd, error = %e, "command failed");
            (StatusCode::BAD_REQUEST, Json(json!({ "error": e })))
        }
    }
}

/// Reset all server state for test isolation.
async fn handle_reset(AxumState(server): AxumState<SharedServer>) -> impl IntoResponse {
    tracing::info!("resetting server state");

    // A logout when one is unlocked; while locked the Host refuses it,
    // which is fine here.
    let _ = host_call::call::<Value>(&server.state, IpcRequest::Logout).await;

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

/// A command's answer: a Host call's `Done`, or the server's own value for
/// the commands not yet ported.
enum Answer {
    Host(Done<Value>),
    Local(Value),
}

/// Route a command: a Host call, or the server's own answer.
async fn dispatch(server: &ServerState, cmd: &str, args: &Value) -> Result<Answer, CommandError> {
    match local(cmd) {
        Some(answer) => answer.map(Answer::Local),
        None => host(server, cmd, args).await.map(Answer::Host),
    }
}

/// The Host calls (plan E1.1).
async fn host(server: &ServerState, cmd: &str, args: &Value) -> Result<Done<Value>, CommandError> {
    let state = &server.state;
    match cmd {
        // ── Host calls (plan E1.1) ───────────────────────────────────
        "create_identity" => {
            let request = IpcRequest::IdentityCreate {
                display_name: arg_str_opt(args, "displayName"),
                passphrase: arg_str(args, "passphrase")?,
            };
            host_call::call(state, request).await
        }
        "login" => {
            let request = IpcRequest::Unlock {
                identity: arg_str(args, "publicKey")?,
                passphrase: arg_str(args, "passphrase")?,
            };
            host_call::call(state, request).await
        }
        "list_identities" => host_call::call(state, IpcRequest::IdentityList).await,
        "delete_identity" => {
            let request = IpcRequest::IdentityDelete {
                identity: arg_str(args, "publicKey")?,
            };
            host_call::call(state, request).await
        }
        "get_identity" => host_call::call(state, IpcRequest::IdentityShow).await,
        "logout" => host_call::call(state, IpcRequest::Logout).await,
        "set_status" => {
            let request = IpcRequest::PresenceSet {
                status: arg_parse(args, "status")?,
            };
            host_call::call(state, request).await
        }
        "set_status_message" => {
            let request = IpcRequest::PresenceSetMessage {
                message: arg_str(args, "message")?,
            };
            host_call::call(state, request).await
        }
        "set_nickname" => {
            let request = IpcRequest::ProfileSetNickname {
                display_name: arg_str(args, "nickname")?,
            };
            host_call::call(state, request).await
        }
        "set_avatar" => {
            let request = IpcRequest::ProfileSetAvatar {
                image: arg_parse(args, "avatarData")?,
                kind: arg_parse(args, "kind")?,
            };
            host_call::call(state, request).await
        }
        "get_avatar" => {
            let request = IpcRequest::AvatarGet {
                public_key: arg_str(args, "publicKey")?,
            };
            host_call::call(state, request).await
        }
        "get_game_status" => {
            let presence: Done<rekindle_types::dto::presence::OwnPresenceView> =
                host_call::call(state, IpcRequest::PresenceGet).await?;
            Ok(Done {
                value: serde_json::to_value(presence.value.game)
                    .map_err(|e| CommandError::internal(e.to_string()))?,
                undo: presence.undo,
            })
        }
        "lifecycle_current" => {
            let status: Done<rekindle_types::display::StatusSnapshot> =
                host_call::call(state, IpcRequest::Status).await?;
            Ok(Done {
                value: Value::String(status.value.state),
                undo: status.undo,
            })
        }
        "intent_commit" => {
            let request = IpcRequest::IntentCommit {
                intent_id: arg_parse(args, "intentId")?,
                proof: arg_parse(args, "proof")?,
            };
            host_call::call(state, request).await
        }
        "intent_cancel" => {
            let request = IpcRequest::IntentCancel {
                intent_id: arg_parse(args, "intentId")?,
            };
            let done: Done<Value> = host_call::call(state, request).await?;
            Ok(Done {
                value: serde_json::to_value(CommandError::Cancelled)
                    .map_err(|e| CommandError::internal(e.to_string()))?,
                undo: done.undo,
            })
        }
        "get_friends" => host_call::call(state, IpcRequest::FriendList).await,
        "get_pending_requests" => host_call::call(state, IpcRequest::FriendRequests).await,
        other => Err(bad_request(format!("unknown command: {other}"))),
    }
}

/// The commands not yet ported to the Host (the window's mount reads), or
/// `None` for a Host call.
fn local(cmd: &str) -> Option<Result<Value, CommandError>> {
    let value = match cmd {
        // ── Communities ──────────────────────────────────────────────
        "get_community_details" | "get_communities" => json!([]),

        // ── Settings ─────────────────────────────────────────────────
        "get_preferences" => json!({
            "notificationsEnabled": true,
            "notificationSound": true,
            "startMinimized": false,
            "autoStart": false,
            "gameDetectionEnabled": true,
            "gameScanIntervalSecs": 30,
            "readReceiptsEnabled": false
        }),

        // ── Game / Network ───────────────────────────────────────────
        "get_network_status" => json!({
            "isAttached": false,
            "publicInternetReady": false,
            "hasRoute": false,
            "mediaRoute": "idle",
            "profileDhtKey": Value::Null,
            "friendListDhtKey": Value::Null,
        }),

        // ── No-op commands (windows, voice, the updater) ─────────────
        "show_buddy_list"
        | "open_chat_window"
        | "open_settings_window"
        | "open_community_window"
        | "open_profile_window"
        | "set_mute"
        | "set_deafen"
        | "check_for_updates" => Value::Null,

        _ => return None,
    };
    Some(Ok(value))
}

fn bad_request(message: String) -> CommandError {
    CommandError::Failed {
        code: 400,
        message,
        remediation: None,
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, CommandError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| bad_request(format!("missing required argument: {key}")))
}

fn arg_parse<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, CommandError> {
    let value = args
        .get(key)
        .cloned()
        .ok_or_else(|| bad_request(format!("missing required argument: {key}")))?;
    serde_json::from_value(value).map_err(|e| bad_request(format!("argument {key}: {e}")))
}

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| if v.is_null() { None } else { v.as_str() })
        .map(String::from)
}
