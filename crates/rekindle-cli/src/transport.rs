//! Daemon client for the CLI and TUI.
//!
//! `DaemonClient` connects to the rekindle-node daemon over the Noise IK
//! encrypted IPC bus. CLI commands use `request_ok()` for request-response.
//! The TUI calls `take_event_receiver()` to get a typed stream of
//! `SubscriptionEvent`s for real-time rendering.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_node::ipc::{
    self,
    client::BusClient,
    protocol::{IpcRequest, IpcResponse},
    message::SecurityLevel,
};
use rekindle_types::subscription_events::SubscriptionEvent;

/// Default RPC timeout — 5 seconds.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Long timeout for operations involving Veilid network I/O.
const LONG_TIMEOUT: Duration = Duration::from_secs(180);

/// The CLI/TUI's connection to the rekindle-node daemon.
///
/// Two consumption modes:
/// - **CLI one-shot**: `request_ok()` for request-response, never subscribes.
/// - **TUI persistent**: `take_event_receiver()` at startup, sends `Subscribe`,
///   receives typed `SubscriptionEvent`s in real-time.
pub struct DaemonClient {
    client: Arc<BusClient>,
    /// Typed event receiver — taken once by the TUI via `take_event_receiver()`.
    event_rx: Option<mpsc::UnboundedReceiver<SubscriptionEvent>>,
}

impl DaemonClient {
    /// Connect to the running daemon.
    pub async fn connect() -> anyhow::Result<Self> {
        let socket_path = ipc::socket_path()
            .map_err(|e| anyhow::anyhow!("cannot resolve daemon socket path: {e}"))?;

        if !socket_path.exists() {
            anyhow::bail!(
                "daemon not running (socket not found at {})\n\
                 start the daemon: rekindle node start",
                socket_path.display()
            );
        }

        let server_pub = ipc::noise_keys::read_bus_public_key()
            .await
            .context("daemon is not running (no bus public key found)")?;

        // Ephemeral keypair: each CLI invocation gets a fresh X25519 key for the
        // Noise IK handshake. CLI clients are stateless — no persistent IPC identity.
        let client_keypair = ipc::generate_keypair()
            .map_err(|e| anyhow::anyhow!("ephemeral keypair generation failed: {e}"))?;

        let sender_id = Uuid::now_v7();
        let mut client = BusClient::connect_with_retry(
            sender_id,
            &socket_path,
            &server_pub,
            client_keypair.as_inner(),
            3,
            Duration::from_millis(500),
        )
        .await
        .map_err(|e| anyhow::anyhow!("daemon connection failed: {e}"))?;

        // Take the event receiver before wrapping in Arc (Arc prevents &mut access)
        let event_rx = client.take_event_receiver();

        Ok(Self {
            client: Arc::new(client),
            event_rx,
        })
    }

    /// Take the subscription event receiver. Can only be called once.
    ///
    /// The TUI calls this at startup to get the typed event stream.
    /// CLI one-shot commands never call this.
    pub fn take_event_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<SubscriptionEvent>> {
        self.event_rx.take()
    }

    /// Subscribe to all events from the daemon.
    ///
    /// Sends `IpcRequest::Subscribe` with a wildcard filter. The server's
    /// EventRouter registers this connection for all event categories.
    pub async fn subscribe_all(&self) -> anyhow::Result<()> {
        use rekindle_types::subscription_events::SubscriptionFilter;
        let response = self.request(IpcRequest::Subscribe {
            filters: vec![SubscriptionFilter::all()],
        }).await?;
        match response {
            IpcResponse::Ok(_) => {
                tracing::info!("subscribed to all daemon events");
                Ok(())
            }
            IpcResponse::Error { code, message, .. } => {
                anyhow::bail!("subscribe failed ({code}): {message}")
            }
            IpcResponse::Event(_) => {
                // Subscribe likely succeeded, event arrived in the response slot
                tracing::warn!("event during subscribe handshake — subscribe assumed successful");
                Ok(())
            }
        }
    }

    /// Subscribe with a community-scoped filter.
    ///
    /// Narrows the event stream to only events from the given community.
    /// The daemon's EventRouter enforces this server-side.
    pub async fn subscribe_scoped(&self, community: &str) -> anyhow::Result<()> {
        use rekindle_types::subscription_events::SubscriptionFilter;
        let response = self.request(IpcRequest::Subscribe {
            filters: vec![SubscriptionFilter::community(community.to_string())],
        }).await?;
        match response {
            IpcResponse::Error { code, message, .. } => {
                anyhow::bail!("subscribe_scoped failed ({code}): {message}")
            }
            IpcResponse::Ok(_) | IpcResponse::Event(_) => Ok(()),
        }
    }

    /// Send a request and return the raw `IpcResponse`.
    pub async fn request(&self, request: IpcRequest) -> anyhow::Result<IpcResponse> {
        let timeout = request_timeout(&request);
        self.client
            .request(request, SecurityLevel::Open, timeout)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("timed out") {
                    anyhow::anyhow!(crate::error::CliError::Timeout(
                        format!("no response within {}s", timeout.as_secs())
                    ))
                } else if msg.contains("connection closed") || msg.contains("channel closed") {
                    anyhow::anyhow!(crate::error::CliError::Daemon {
                        code: 503,
                        message: "daemon connection lost".into(),
                    })
                } else {
                    anyhow::anyhow!(msg)
                }
            })
    }

    /// Send a request and unwrap success as `serde_json::Value`.
    pub async fn request_ok(&self, request: IpcRequest) -> anyhow::Result<serde_json::Value> {
        match self.request(request).await? {
            IpcResponse::Ok(value) => Ok(value),
            IpcResponse::Error { code, message, remediation } => {
                let cli_err = match code {
                    403 => crate::error::CliError::Auth(message),
                    503 => crate::error::CliError::NotInitialized(message),
                    408 => crate::error::CliError::Timeout(message),
                    _ => crate::error::CliError::Daemon { code, message },
                };
                if let Some(hint) = remediation {
                    tracing::debug!(hint, "daemon remediation hint");
                }
                Err(anyhow::anyhow!(cli_err))
            }
            IpcResponse::Event(_) => {
                tracing::warn!("event leaked into request-response path");
                Err(anyhow::anyhow!(crate::error::CliError::Daemon {
                    code: 500,
                    message: "unexpected event in request-response flow".into(),
                }))
            }
        }
    }

    /// Gracefully shut down the client connection.
    pub async fn shutdown(self) {
        match Arc::try_unwrap(self.client) {
            Ok(client) => client.shutdown().await,
            Err(arc) => {
                tracing::debug!(
                    refs = Arc::strong_count(&arc),
                    "client shutdown with outstanding refs — dropping"
                );
                drop(arc);
            }
        }
    }
}

fn request_timeout(request: &IpcRequest) -> Duration {
    match request {
        IpcRequest::IdentityCreate { .. }
        | IpcRequest::IdentityRotate
        | IpcRequest::IdentityDestroy { .. }
        | IpcRequest::IdentityWipe { .. }
        | IpcRequest::CommunityCreate { .. }
        | IpcRequest::CommunityJoin { .. }
        | IpcRequest::FriendAdd { .. }
        | IpcRequest::FriendAccept { .. }
        | IpcRequest::Unlock { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::DmSend { .. }
        | IpcRequest::DmInbox { .. }
        | IpcRequest::ChannelHistory { .. } => LONG_TIMEOUT,
        _ => DEFAULT_TIMEOUT,
    }
}
