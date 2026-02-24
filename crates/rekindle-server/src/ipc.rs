use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::audit;
use crate::community_host;
use crate::server_state::ServerState;

/// JSON-RPC request from the Tauri client to the server daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum IpcRequest {
    /// Start hosting a community.
    HostCommunity {
        community_id: String,
        dht_record_key: String,
        owner_keypair_hex: String,
        name: String,
        /// Pseudonym key of the community creator (registered as first member/owner).
        creator_pseudonym_key: String,
        /// Display name of the creator.
        creator_display_name: String,
    },
    /// Stop hosting a community.
    UnhostCommunity { community_id: String },
    /// List currently hosted communities.
    ListHosted,
    /// Get server status.
    GetStatus,
    /// Shut down the server.
    Shutdown,
    /// Forward a community RPC request through IPC (bypasses Veilid).
    CommunityRpc {
        community_id: String,
        sender_pseudonym_key: String,
        request_json: String,
    },
    /// Generate an invite code for a community.
    CreateInvite {
        community_id: String,
        max_uses: Option<u32>,
        expires_in_seconds: Option<u64>,
    },
}

/// JSON-RPC response from the server daemon to the Tauri client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum IpcResponse {
    /// Generic success.
    Ok,
    /// List of hosted communities.
    Hosted {
        communities: Vec<HostedCommunityInfo>,
    },
    /// Server status.
    Status {
        uptime_secs: u64,
        community_count: usize,
        veilid_attached: bool,
        server_public_key: String,
    },
    /// Error.
    Error { message: String },
    /// Response carrying a community RPC result (JSON-encoded `CommunityResponse`).
    RpcResult { response_json: String },
    /// Invite code generated successfully.
    InviteCreated { code: String, signature: String },
}

/// Summary info for a hosted community.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedCommunityInfo {
    pub community_id: String,
    pub dht_record_key: String,
    pub member_count: usize,
    pub has_route: bool,
}

/// Platform-specific async stream type for IPC.
#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
type IpcStream = tokio::net::TcpStream;

/// Start the IPC listener using **async tokio tasks**.
///
/// On Unix: binds a `tokio::net::UnixListener` on the given socket path.
/// On Windows: binds a `tokio::net::TcpListener` on `127.0.0.1:19280`.
///
/// Each incoming connection is handled in a spawned tokio task, and all
/// dispatch is fully async — no `block_on()` bridges, no OS-level socket
/// timeouts, no EAGAIN.
pub async fn start_ipc_listener(
    socket_path: String,
    state: Arc<ServerState>,
    shutdown_tx: mpsc::Sender<()>,
) {
    #[cfg(unix)]
    {
        // Remove stale socket file if it exists
        let _ = std::fs::remove_file(&socket_path);

        let listener = match tokio::net::UnixListener::bind(&socket_path) {
            std::result::Result::Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, path = %socket_path, "failed to bind IPC socket");
                return;
            }
        };

        tracing::info!(path = %socket_path, "IPC listener started (async)");

        loop {
            match listener.accept().await {
                std::result::Result::Ok((stream, _addr)) => {
                    let conn_state = Arc::clone(&state);
                    let conn_shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(handle_connection(stream, conn_state, conn_shutdown_tx));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "IPC accept error");
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:19280").await {
            std::result::Result::Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "failed to bind IPC TCP listener on port 19280");
                return;
            }
        };

        tracing::info!("IPC listener started on 127.0.0.1:19280 (async)");

        loop {
            match listener.accept().await {
                std::result::Result::Ok((stream, _addr)) => {
                    let conn_state = Arc::clone(&state);
                    let conn_shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(handle_connection(stream, conn_state, conn_shutdown_tx));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "IPC accept error");
                }
            }
        }
    }
}

/// Handle a single IPC connection: read newline-delimited JSON requests,
/// process them, and write JSON responses.
async fn handle_connection(
    stream: IpcStream,
    state: Arc<ServerState>,
    shutdown_tx: mpsc::Sender<()>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(reader).lines();

    while let std::result::Result::Ok(Some(line)) = lines.next_line().await {
        let request: IpcRequest = match serde_json::from_str(&line) {
            std::result::Result::Ok(r) => r,
            Err(e) => {
                let resp = IpcResponse::Error {
                    message: format!("invalid request: {e}"),
                };
                if write_response(&mut writer, &resp).await.is_err() {
                    break;
                }
                continue;
            }
        };

        let response = handle_ipc_request(&state, request, &shutdown_tx).await;
        if write_response(&mut writer, &response).await.is_err() {
            break;
        }
    }
}

/// Serialize and write a JSON response followed by a newline.
async fn write_response<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &IpcResponse,
) -> Result<(), ()> {
    let mut buf = serde_json::to_vec(response).unwrap_or_default();
    buf.push(b'\n');
    if let Err(e) = writer.write_all(&buf).await {
        tracing::warn!(error = %e, "failed to write IPC response");
        return Err(());
    }
    if let Err(e) = writer.flush().await {
        tracing::warn!(error = %e, "failed to flush IPC response");
        return Err(());
    }
    Ok(())
}

/// Process a single IPC request and return the response.
///
/// All handlers run as native async — no `block_on()` bridging. Sync handlers
/// (ListHosted, UnhostCommunity, CreateInvite) execute inline without yielding.
/// Async handlers (HostCommunity, GetStatus, CommunityRpc) `.await` directly.
async fn handle_ipc_request(
    state: &Arc<ServerState>,
    request: IpcRequest,
    shutdown_tx: &mpsc::Sender<()>,
) -> IpcResponse {
    match request {
        IpcRequest::HostCommunity {
            community_id,
            dht_record_key,
            owner_keypair_hex,
            name,
            creator_pseudonym_key,
            creator_display_name,
        } => {
            match community_host::host_community(
                state,
                &community_id,
                &dht_record_key,
                &owner_keypair_hex,
                &name,
                &creator_pseudonym_key,
                &creator_display_name,
            )
            .await
            {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error { message: e },
            }
        }
        IpcRequest::UnhostCommunity { community_id } => {
            community_host::unhost_community(state, &community_id);
            IpcResponse::Ok
        }
        IpcRequest::ListHosted => {
            let hosted = state.hosted.read();
            let communities = hosted
                .values()
                .map(|h| HostedCommunityInfo {
                    community_id: h.community_id.clone(),
                    dht_record_key: h.dht_record_key.clone(),
                    member_count: h.members.len(),
                    has_route: h.route_id.is_some(),
                })
                .collect();
            IpcResponse::Hosted { communities }
        }
        IpcRequest::GetStatus => {
            let now = rekindle_utils::timestamp_secs();
            let uptime_secs = now.saturating_sub(state.started_at);
            let community_count = state.hosted.read().len();
            let veilid_attached = state
                .api
                .get_state()
                .await
                .map(|vs| vs.attachment.state.is_attached())
                .unwrap_or(false);
            IpcResponse::Status {
                uptime_secs,
                community_count,
                veilid_attached,
                server_public_key: state.public_key_hex.clone(),
            }
        }
        IpcRequest::Shutdown => {
            tracing::info!("shutdown requested via IPC");
            let _ = shutdown_tx.send(()).await;
            IpcResponse::Ok
        }
        IpcRequest::CommunityRpc {
            community_id,
            sender_pseudonym_key,
            request_json,
        } => {
            let response_bytes = crate::rpc::handle_community_rpc_direct(
                state,
                &community_id,
                &sender_pseudonym_key,
                &request_json,
            )
            .await;
            match String::from_utf8(response_bytes) {
                Ok(response_json) => IpcResponse::RpcResult { response_json },
                Err(e) => IpcResponse::Error {
                    message: format!("non-UTF8 RPC response: {e}"),
                },
            }
        }
        IpcRequest::CreateInvite {
            community_id,
            max_uses,
            expires_in_seconds,
        } => create_invite(state, &community_id, max_uses, expires_in_seconds),
    }
}

/// Generate a random invite code and persist it to the database.
fn create_invite(
    state: &Arc<ServerState>,
    community_id: &str,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
) -> IpcResponse {
    // Verify the community is hosted
    {
        let hosted = state.hosted.read();
        if !hosted.contains_key(community_id) {
            return IpcResponse::Error {
                message: format!("community {community_id} is not hosted on this server"),
            };
        }
    }

    let code = crate::invite_util::generate_invite_code();

    let now = rekindle_utils::timestamp_secs();
    let expires_at = expires_in_seconds.map(|secs| now + secs);

    {
        let db = crate::db_helpers::lock_db(&state.db);

        if let Err(e) = db.execute(
            "INSERT INTO server_invites (code, community_id, created_by, max_uses, expires_at, created_at) \
             VALUES (?, ?, 'server', ?, ?, ?)",
            rusqlite::params![code, community_id, max_uses, expires_at, now],
        ) {
            return IpcResponse::Error {
                message: format!("failed to create invite: {e}"),
            };
        }
    } // DB lock dropped before audit call

    // Sign the invite code with the server's identity key
    let signature = hex::encode(state.identity.sign(code.as_bytes()).to_bytes());

    tracing::info!(
        code = %code,
        community = %community_id,
        max_uses = ?max_uses,
        expires_at = ?expires_at,
        "created invite code"
    );
    audit::log_action(state, community_id, audit::AuditAction::CreateInvite, "server", None, Some(&code));

    IpcResponse::InviteCreated { code, signature }
}
