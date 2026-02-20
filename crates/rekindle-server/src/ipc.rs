use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

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
    },
    /// Error.
    Error { message: String },
    /// Response carrying a community RPC result (JSON-encoded `CommunityResponse`).
    RpcResult { response_json: String },
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

/// Start the IPC listener on a Unix socket.
///
/// Reads newline-delimited JSON requests and writes JSON responses.
pub async fn start_ipc_listener(
    socket_path: &str,
    state: Arc<ServerState>,
    shutdown_tx: mpsc::Sender<()>,
) {
    // Remove stale socket file if it exists
    let _ = std::fs::remove_file(socket_path);

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, path = %socket_path, "failed to bind IPC socket");
            return;
        }
    };

    tracing::info!(path = %socket_path, "IPC listener started");

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut lines = BufReader::new(reader).lines();

                    while let Ok(Some(line)) = lines.next_line().await {
                        let request: IpcRequest = match serde_json::from_str(&line) {
                            Ok(r) => r,
                            Err(e) => {
                                let resp = IpcResponse::Error {
                                    message: format!("invalid request: {e}"),
                                };
                                let mut buf = serde_json::to_vec(&resp).unwrap_or_default();
                                buf.push(b'\n');
                                if let Err(e) = writer.write_all(&buf).await {
                                    tracing::warn!(error = %e, "failed to write IPC error response");
                                    break;
                                }
                                continue;
                            }
                        };

                        let response = handle_ipc_request(&state, request, &shutdown_tx).await;
                        let mut buf = serde_json::to_vec(&response).unwrap_or_default();
                        buf.push(b'\n');
                        if let Err(e) = writer.write_all(&buf).await {
                            tracing::warn!(error = %e, "failed to write IPC response");
                            break;
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "IPC accept error");
            }
        }
    }
}

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
            let now = timestamp_now();
            let uptime_secs = now.saturating_sub(state.started_at);
            let community_count = state.hosted.read().len();
            let veilid_attached = match state.api.get_state().await {
                Ok(vs) => vs.attachment.state.is_attached(),
                Err(_) => false,
            };
            IpcResponse::Status {
                uptime_secs,
                community_count,
                veilid_attached,
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
    }
}

fn timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
