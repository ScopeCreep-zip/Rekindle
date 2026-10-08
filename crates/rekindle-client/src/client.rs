//! The frontend's connection to `rekindled` over the Noise IK bus.
//!
//! Two consumption modes:
//! - **one-shot**: [`DaemonClient::request_ok`] per command;
//! - **streaming**: [`DaemonClient::subscribe_all`], then the typed events
//!   from [`DaemonClient::take_event_receiver`] (the TUI, and the CLI's
//!   `watch` commands).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_ipc::client::BusClient;
use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::IpcError;
use rekindle_types::subscription_events::{SubscriptionEvent, SubscriptionFilter};

use crate::ClientError;

/// Default request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for requests that wait on Veilid network I/O.
const LONG_TIMEOUT: Duration = Duration::from_secs(180);

/// A connection to the daemon.
pub struct DaemonClient {
    client: Arc<BusClient>,
    /// Typed event stream, taken once by a streaming consumer.
    event_rx: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<SubscriptionEvent>>>,
}

impl DaemonClient {
    /// Connect to a running daemon.
    ///
    /// # Errors
    /// [`ClientError::NotRunning`] when there is no socket; otherwise the
    /// bus key or the handshake failed.
    pub async fn connect() -> Result<Self, ClientError> {
        let socket =
            rekindle_ipc::socket_path().map_err(|e| ClientError::Connect(e.to_string()))?;
        if !socket.exists() {
            return Err(ClientError::NotRunning { socket });
        }
        let server_pub = rekindle_ipc::noise_keys::read_bus_public_key()
            .await
            .map_err(|e| ClientError::Connect(format!("no bus public key: {e}")))?;
        // A fresh ephemeral Noise key per connection; clearance for a
        // persistent client key arrives with the vault (plan D1).
        let keypair = rekindle_ipc::generate_keypair().map_err(ClientError::Ipc)?;
        let mut client = BusClient::connect_with_retry(
            Uuid::now_v7(),
            &socket,
            &server_pub,
            keypair.as_inner(),
            3,
            Duration::from_millis(500),
        )
        .await
        .map_err(|e| ClientError::Connect(e.to_string()))?;
        let event_rx = client.take_event_receiver();
        Ok(Self {
            client: Arc::new(client),
            event_rx: parking_lot::Mutex::new(event_rx),
        })
    }

    /// Take the typed event stream. Returns `None` after the first call.
    pub fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<SubscriptionEvent>> {
        self.event_rx.lock().take()
    }

    /// Subscribe this connection to every event category.
    ///
    /// # Errors
    /// The request failed or the daemon refused it.
    pub async fn subscribe_all(&self) -> Result<(), ClientError> {
        self.subscribe(vec![SubscriptionFilter::all()]).await
    }

    /// Subscribe this connection to the events matching `filters`.
    ///
    /// # Errors
    /// The request failed or the daemon refused it.
    pub async fn subscribe(&self, filters: Vec<SubscriptionFilter>) -> Result<(), ClientError> {
        self.request_ok(IpcRequest::Subscribe { filters })
            .await
            .map(drop)
    }

    /// Send a request and return the daemon's response.
    ///
    /// # Errors
    /// Timeout, a dropped connection, or another bus failure.
    pub async fn request(&self, request: IpcRequest) -> Result<IpcResponse, ClientError> {
        let timeout = request_timeout(&request);
        self.client
            .request(request, SecurityLevel::Open, timeout)
            .await
            .map_err(|e| match e {
                IpcError::RequestTimeout { .. } => ClientError::Timeout {
                    secs: timeout.as_secs(),
                },
                IpcError::ConnectionClosed | IpcError::OutboundClosed => {
                    ClientError::ConnectionLost
                }
                other => ClientError::Ipc(other),
            })
    }

    /// Send a request and unwrap a successful answer.
    ///
    /// # Errors
    /// As [`Self::request`], plus [`ClientError::Daemon`] for an error
    /// answer. Events never travel in the response slot, so one there is a
    /// protocol violation.
    pub async fn request_ok(&self, request: IpcRequest) -> Result<serde_json::Value, ClientError> {
        match self.request(request).await? {
            IpcResponse::Ok(value) => Ok(value),
            IpcResponse::Error {
                code,
                message,
                remediation,
            } => Err(ClientError::Daemon {
                code,
                message,
                remediation,
            }),
            IpcResponse::Event(_) => Err(ClientError::Protocol("event in a response")),
        }
    }

    /// Close the connection, flushing anything still queued.
    pub async fn shutdown(self) {
        match Arc::try_unwrap(self.client) {
            Ok(client) => client.shutdown().await,
            Err(shared) => {
                tracing::debug!(
                    refs = Arc::strong_count(&shared),
                    "client shutdown with outstanding refs — dropping"
                );
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
