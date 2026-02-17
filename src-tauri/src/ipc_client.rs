use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

// Platform-specific stream types
#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(windows)]
use std::net::TcpStream;

/// JSON-RPC request to the rekindle-server daemon (mirrors ipc.rs on the server).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum IpcRequest {
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
    UnhostCommunity {
        community_id: String,
    },
    ListHosted,
    GetStatus,
    Shutdown,
    /// Forward a community RPC request through the IPC socket (bypasses Veilid).
    CommunityRpc {
        community_id: String,
        sender_pseudonym_key: String,
        request_json: String,
    },
}

/// JSON-RPC response from the rekindle-server daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum IpcResponse {
    Ok,
    Hosted {
        communities: Vec<HostedCommunityInfo>,
    },
    Status {
        uptime_secs: u64,
        community_count: usize,
        veilid_attached: bool,
    },
    Error {
        message: String,
    },
    /// Response carrying a community RPC result (JSON-encoded `CommunityResponse`).
    RpcResult {
        response_json: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedCommunityInfo {
    pub community_id: String,
    pub dht_record_key: String,
    pub member_count: usize,
    pub has_route: bool,
}

/// Synchronous IPC client that talks to the rekindle-server.
///
/// On Unix: Uses Unix domain sockets.
/// On Windows: Uses TCP localhost (the server must listen on the TCP port).
///
/// Uses blocking I/O since the server daemon may take a moment to respond.
/// All methods are designed to be called from `spawn_blocking`.
pub struct IpcClient {
    #[cfg(unix)]
    stream: BufReader<UnixStream>,
    #[cfg(windows)]
    stream: BufReader<TcpStream>,
}

impl IpcClient {
    /// Connect to the server with default timeouts (10s read, 5s write).
    pub fn connect(socket_path: &Path) -> Result<Self, String> {
        Self::connect_with_timeout(socket_path, Duration::from_secs(10))
    }

    /// Connect with a custom read timeout.
    ///
    /// `HostCommunity` can take 60+ seconds (Veilid attachment + DHT record open),
    /// so callers that issue slow commands should use a longer timeout.
    #[cfg(unix)]
    pub fn connect_with_timeout(socket_path: &Path, read_timeout: Duration) -> Result<Self, String> {
        let stream = UnixStream::connect(socket_path)
            .map_err(|e| format!("failed to connect to server socket at {}: {e}", socket_path.display()))?;
        stream
            .set_read_timeout(Some(read_timeout))
            .map_err(|e| format!("failed to set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("failed to set write timeout: {e}"))?;

        Ok(Self {
            stream: BufReader::new(stream),
        })
    }

    /// Connect with a custom read timeout (Windows - uses TCP localhost).
    ///
    /// On Windows, the socket_path is ignored; we connect to TCP port 19280.
    #[cfg(windows)]
    pub fn connect_with_timeout(_socket_path: &Path, read_timeout: Duration) -> Result<Self, String> {
        // Windows uses TCP localhost instead of Unix domain sockets
        let addr = "127.0.0.1:19280";
        let stream = TcpStream::connect(addr)
            .map_err(|e| format!("failed to connect to server at {addr}: {e}"))?;
        stream
            .set_read_timeout(Some(read_timeout))
            .map_err(|e| format!("failed to set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("failed to set write timeout: {e}"))?;

        Ok(Self {
            stream: BufReader::new(stream),
        })
    }

    /// Send a request and read the response.
    pub fn send(&mut self, request: &IpcRequest) -> Result<IpcResponse, String> {
        let mut buf = serde_json::to_vec(request)
            .map_err(|e| format!("failed to serialize IPC request: {e}"))?;
        buf.push(b'\n');

        self.stream
            .get_mut()
            .write_all(&buf)
            .map_err(|e| format!("failed to write to server socket: {e}"))?;

        let mut response_line = String::new();
        self.stream
            .read_line(&mut response_line)
            .map_err(|e| format!("failed to read from server socket: {e}"))?;

        serde_json::from_str::<IpcResponse>(response_line.trim())
            .map_err(|e| format!("failed to parse server response: {e}"))
    }
}

/// The default socket path for the rekindle-server daemon.
///
/// On Unix: Returns a path to a Unix socket in the temp directory.
/// On Windows: Returns a placeholder path (actual connection uses TCP port 19280).
pub fn default_socket_path() -> PathBuf {
    std::env::temp_dir().join("rekindle-server.sock")
}

/// Send a `HostCommunity` command to the server (blocking — call from `spawn_blocking`).
///
/// Retries connection up to `max_retries` times with a delay to allow the server
/// to finish starting up.
#[allow(clippy::too_many_arguments)]
pub fn host_community_blocking(
    socket_path: &Path,
    community_id: &str,
    dht_record_key: &str,
    owner_keypair_hex: &str,
    name: &str,
    creator_pseudonym_key: &str,
    creator_display_name: &str,
    max_retries: u32,
) -> Result<(), String> {
    let request = IpcRequest::HostCommunity {
        community_id: community_id.to_string(),
        dht_record_key: dht_record_key.to_string(),
        owner_keypair_hex: owner_keypair_hex.to_string(),
        name: name.to_string(),
        creator_pseudonym_key: creator_pseudonym_key.to_string(),
        creator_display_name: creator_display_name.to_string(),
    };

    // Use a 90-second read timeout — host_community can take 60+ seconds
    // (30s Veilid attachment wait + DHT record open with exponential backoff).
    let host_timeout = Duration::from_secs(90);

    for attempt in 1..=max_retries {
        match IpcClient::connect_with_timeout(socket_path, host_timeout) {
            Ok(mut client) => match client.send(&request) {
                Ok(IpcResponse::Ok) => {
                    tracing::info!(
                        community = %community_id,
                        "HostCommunity IPC succeeded"
                    );
                    return Ok(());
                }
                Ok(IpcResponse::Error { message }) => {
                    return Err(format!("server rejected HostCommunity: {message}"));
                }
                Ok(other) => {
                    tracing::debug!(?other, "unexpected IPC response for HostCommunity");
                    return Ok(()); // Non-error response — treat as success
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "IPC send failed for HostCommunity");
                    if attempt < max_retries {
                        std::thread::sleep(Duration::from_millis(1000 * u64::from(attempt)));
                    }
                }
            },
            Err(e) => {
                if attempt < max_retries {
                    tracing::debug!(
                        attempt,
                        max = max_retries,
                        error = %e,
                        "server not ready — retrying"
                    );
                    std::thread::sleep(Duration::from_millis(500 * u64::from(attempt)));
                } else {
                    return Err(format!(
                        "failed to connect to server after {max_retries} attempts: {e}"
                    ));
                }
            }
        }
    }

    Err("exhausted retries".to_string())
}

/// Send a `GetStatus` command to the server (blocking).
///
/// Returns `(uptime_secs, community_count, veilid_attached)` on success.
pub fn get_status_blocking(socket_path: &Path) -> Result<(u64, usize, bool), String> {
    let mut client = IpcClient::connect(socket_path)?;
    match client.send(&IpcRequest::GetStatus) {
        Ok(IpcResponse::Status {
            uptime_secs,
            community_count,
            veilid_attached,
        }) => Ok((uptime_secs, community_count, veilid_attached)),
        Ok(IpcResponse::Error { message }) => Err(message),
        Ok(other) => Err(format!("unexpected response to GetStatus: {other:?}")),
        Err(e) => Err(e),
    }
}

/// Send a `Shutdown` command to the server (blocking).
pub fn shutdown_server_blocking(socket_path: &Path) -> Result<(), String> {
    let mut client = IpcClient::connect(socket_path)?;
    match client.send(&IpcRequest::Shutdown) {
        Ok(IpcResponse::Error { message }) => Err(message),
        _ => Ok(()),
    }
}

/// Send a `CommunityRpc` request through the socket (blocking).
///
/// Bypasses Veilid entirely — used for hosted communities where the server
/// process is local. Returns the JSON-encoded `CommunityResponse`.
pub fn community_rpc_blocking(
    socket_path: &Path,
    community_id: &str,
    sender_pseudonym_key: &str,
    request_json: &str,
) -> Result<String, String> {
    let mut client = IpcClient::connect(socket_path)?;
    let request = IpcRequest::CommunityRpc {
        community_id: community_id.to_string(),
        sender_pseudonym_key: sender_pseudonym_key.to_string(),
        request_json: request_json.to_string(),
    };
    match client.send(&request) {
        Ok(IpcResponse::RpcResult { response_json }) => Ok(response_json),
        Ok(IpcResponse::Error { message }) => Err(format!("server error: {message}")),
        Ok(other) => Err(format!("unexpected IPC response: {other:?}")),
        Err(e) => Err(e),
    }
}
