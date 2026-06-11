//! Typed IPC client for the Rekindle daemon.
//!
//! Wraps transport-ipc's `IpcClient` with application-level types.
//! CLI, TUI, Tauri, and agent SDKs depend on this crate — not on
//! transport-ipc directly. The transport boundary is invisible to
//! consumers.
//!
//! Request-response uses `IpcClient::request_reply()` which delivers
//! the reply payload via a per-request oneshot, bypassing the shared
//! `recv()` channel entirely. No double-decode, no frame race,
//! concurrent-safe at 200K agents per node.
//!
//! Subscription events use a background mux task that owns `recv()`
//! exclusively. The mux filters for PUBLISH frames and deserializes
//! `SubscriptionEvent` directly from the application payload — the
//! transport already decoded the wire format.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use rekindle_keys as keys;
use rekindle_transport_ipc::v3::client::IpcClient;
use rekindle_transport_ipc::v3::context::SessionConfig;
use rekindle_transport_ipc::v3::session::handshake::HandshakeConfig;
use rekindle_transport_ipc::v3::wire::frame_kind::DatagramKind;
pub use rekindle_types::daemon::{AgentType, DaemonRequest, DaemonResponse, ReadContext};
use rekindle_types::subscription_events::{SubscriptionEvent, SubscriptionFilter};

/// Default RPC timeout — 5 seconds for quick operations.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Long timeout for operations involving Veilid network I/O.
const LONG_TIMEOUT: Duration = Duration::from_secs(180);

/// The typed IPC client for the Rekindle daemon.
///
/// Two consumption modes:
/// - **CLI one-shot**: `request_ok()` for request-response, never subscribes.
/// - **TUI persistent**: `take_event_receiver()` at startup, sends `Subscribe`,
///   receives typed `SubscriptionEvent`s via the returned channel.
pub struct DaemonClient {
    client: Arc<IpcClient>,
}

impl DaemonClient {
    /// Connect to the running daemon.
    ///
    /// Uses an ephemeral X25519 keypair — CLI clients are stateless.
    /// The server's static public key is read from the daemon's key file.
    pub async fn connect() -> anyhow::Result<Self> {
        let socket_path = keys::socket_path()
            .map_err(|e| anyhow::anyhow!("cannot resolve daemon socket path: {e}"))?;

        if !socket_path.exists() {
            anyhow::bail!(
                "daemon not running (socket not found at {})\n\
                 start the daemon: rekindle node start",
                socket_path.display()
            );
        }

        let server_pub = keys::read_bus_public_key()
            .await
            .map_err(|e| anyhow::anyhow!("daemon is not running (no bus public key found): {e}"))?;

        let client_keypair = keys::generate_keypair()
            .map_err(|e| anyhow::anyhow!("ephemeral keypair generation failed: {e}"))?;

        let client = IpcClient::connect(
            &socket_path,
            &server_pub,
            client_keypair.as_inner(),
            SessionConfig::default(),
            HandshakeConfig::default(),
        ).await.map_err(|e| anyhow::anyhow!("daemon connection failed: {e}"))?;

        info!(path = %socket_path.display(), "connected to daemon");

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Take the subscription event receiver. Can only be called once.
    ///
    /// Spawns a background mux task that owns `recv()` exclusively.
    /// Filters for PUBLISH frames and deserializes `SubscriptionEvent`
    /// directly from `InboundFrame.payload` — the transport already
    /// decoded the wire format, so the payload IS the serialized
    /// application bytes. Deserialize via `SubscriptionEvent::from_bytes()`.
    ///
    /// Since `request_reply()` uses per-request oneshots that bypass
    /// `recv()` entirely, there is no race between the mux task and
    /// the request path.
    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<SubscriptionEvent>> {
        let (tx, rx) = mpsc::channel(4096);
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            debug!("event mux task started");
            loop {
                let frame = match client.recv().await {
                    Some(f) => f,
                    None => break,
                };
                if frame.kind == DatagramKind::Publish as u8 {
                    match SubscriptionEvent::from_bytes(&frame.payload) {
                        Ok(event) => {
                            if tx.send(event).await.is_err() {
                                debug!("event mux: consumer dropped — exiting");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "event mux: SubscriptionEvent deserialization failed");
                        }
                    }
                }
            }
            debug!("event mux task exiting — connection closed");
        });
        Some(rx)
    }

    /// Subscribe to all events from the daemon.
    pub async fn subscribe_all(&self) -> anyhow::Result<()> {
        let response = self.request(DaemonRequest::Subscribe {
            filters: vec![SubscriptionFilter::all()],
        }).await?;
        match response {
            DaemonResponse::Ok(_) => {
                info!("subscribed to all daemon events");
                Ok(())
            }
            DaemonResponse::Error { code, message, .. } => {
                anyhow::bail!("subscribe failed ({code}): {message}")
            }
        }
    }

    /// Subscribe with a community-scoped filter.
    pub async fn subscribe_scoped(&self, community: &str) -> anyhow::Result<()> {
        let response = self.request(DaemonRequest::Subscribe {
            filters: vec![SubscriptionFilter::community(community.to_string())],
        }).await?;
        match response {
            DaemonResponse::Error { code, message, .. } => {
                anyhow::bail!("subscribe_scoped failed ({code}): {message}")
            }
            DaemonResponse::Ok(_) => Ok(()),
        }
    }

    /// Send a request and return the raw `DaemonResponse`.
    ///
    /// Uses `request_reply()` which delivers the reply payload directly
    /// via a per-request oneshot, bypassing the shared `recv()` channel.
    /// Concurrent callers each get their own oneshot — no serialization
    /// needed, no frame race possible.
    pub async fn request(&self, request: DaemonRequest) -> anyhow::Result<DaemonResponse> {
        let timeout = request_timeout(&request);
        let payload = request.to_bytes()
            .map_err(|e| anyhow::anyhow!("request serialization failed: {e}"))?;

        let reply = self.client.request_reply(&payload, timeout).await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let response = DaemonResponse::from_bytes(&reply.payload)
            .map_err(|e| anyhow::anyhow!("response deserialization failed: {e}"))?;

        Ok(response)
    }

    /// Send a request and unwrap success as `serde_json::Value`.
    pub async fn request_ok(&self, request: DaemonRequest) -> anyhow::Result<serde_json::Value> {
        match self.request(request).await? {
            DaemonResponse::Ok(bytes) => {
                serde_json::from_slice(&bytes)
                    .map_err(|e| anyhow::anyhow!("response payload parse failed: {e}"))
            }
            DaemonResponse::Error { code, message, remediation } => {
                if let Some(ref hint) = remediation {
                    debug!(hint, "daemon remediation hint");
                }
                Err(anyhow::anyhow!(ClientError::Daemon { code, message }))
            }
        }
    }

    /// Gracefully shut down the client connection.
    pub async fn shutdown(self) {
        match Arc::try_unwrap(self.client) {
            Ok(client) => {
                debug!("client shutdown: sending GOODBYE");
                client.shutdown().await;
                debug!("client shutdown: complete");
            }
            Err(arc) => {
                debug!(
                    refs = Arc::strong_count(&arc),
                    "client shutdown with outstanding refs — dropping"
                );
                drop(arc);
            }
        }
    }
}

/// Determine the appropriate timeout for a request.
///
/// Operations involving Veilid network I/O get the long timeout.
/// Local operations (status, lock, unlock) get the default timeout.
///
/// NOTE: This function must be updated when new `DaemonRequest` variants
/// are added that involve network I/O.
fn request_timeout(request: &DaemonRequest) -> Duration {
    match request {
        DaemonRequest::Unlock { .. }
        | DaemonRequest::IdentityCreate { .. }
        | DaemonRequest::IdentityRotate
        | DaemonRequest::IdentityDestroy { .. }
        | DaemonRequest::IdentityWipe { .. }
        | DaemonRequest::CommunityCreate { .. }
        | DaemonRequest::CommunityJoin { .. }
        | DaemonRequest::CommunityLeave { .. }
        | DaemonRequest::FriendAdd { .. }
        | DaemonRequest::FriendAccept { .. }
        | DaemonRequest::ChannelSend { .. }
        | DaemonRequest::DmSend { .. }
        | DaemonRequest::ChannelHistory { .. }
        | DaemonRequest::DmInbox { .. }
        | DaemonRequest::DmThread { .. }
        | DaemonRequest::MekRotate { .. }
        | DaemonRequest::PrekeyReplenish
        | DaemonRequest::BootstrapRespond { .. } => LONG_TIMEOUT,
        _ => DEFAULT_TIMEOUT,
    }
}

// ── Error types ─────────────────────────────────────────────────────

/// Client-side error type for daemon interaction.
///
/// Used by `request_ok()` to convert `DaemonResponse::Error` into a
/// typed error with exit code mapping. Downstream consumers (CLI, TUI)
/// downcast from `anyhow::Error` to match on these variants.
#[derive(Debug)]
pub enum ClientError {
    /// Daemon returned an error response with a typed code.
    Daemon { code: u32, message: String },
    /// Daemon not running or identity not created.
    NotInitialized(String),
    /// Operation timed out waiting for daemon response.
    Timeout(String),
    /// Authentication or authorization failure.
    Auth(String),
    /// IPC connection lost mid-operation.
    ConnectionLost(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Daemon { code, message } => write!(f, "daemon ({code}): {message}"),
            Self::NotInitialized(msg) => write!(f, "not initialized: {msg}"),
            Self::Timeout(msg) => write!(f, "timeout: {msg}"),
            Self::Auth(msg) => write!(f, "auth failed: {msg}"),
            Self::ConnectionLost(msg) => write!(f, "connection lost: {msg}"),
        }
    }
}

impl std::error::Error for ClientError {}
