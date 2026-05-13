//! IPC bus client — connects to the bus server, publishes events,
//! and receives subscribed events over an encrypted Unix domain socket.
//!
//! Two construction modes:
//! - `connect()`: low-level, for ephemeral CLI clients and tests.
//! - `connect_with_retry()`: production, retries with backoff.
//!
//! Adapted from open-sesame `core-ipc/src/client.rs`.


use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::error::{IpcError, Result};
use super::framing::{decode_frame, encode_frame};
use super::message::{Message, MessageContext, SecurityLevel, Timestamp};
use super::noise;
use super::protocol::{BusPayload, IpcRequest, IpcResponse};
use super::transport::{extract_ucred, PeerCredentials};

/// Event channel capacity. If the consumer falls behind by this many events,
/// new events are dropped rather than accumulating unboundedly.
/// 4096 events × ~200 bytes avg = ~800KB buffer.
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// The IPC bus client used by frontends and agents.
pub struct BusClient {
    sender_id: Uuid,
    msg_ctx: MessageContext,
    /// Outbound frames to send to the server.
    outbound_tx: mpsc::Sender<bytes::Bytes>,
    /// Inbound frames received from the server (forwarded requests for daemon subscriber).
    inbound_rx: mpsc::Receiver<bytes::Bytes>,
    /// Typed subscription events from the server's EventRouter.
    /// Taken once by the consumer via `take_event_receiver()`.
    event_rx: Option<mpsc::Receiver<rekindle_types::subscription_events::SubscriptionEvent>>,
    /// Pending request-response waiters, keyed by msg_id.
    pending: Arc<parking_lot::Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>>,
    /// Monotonic epoch for timestamp generation.
    epoch: Instant,
    /// Handle to the multiplexed I/O task.
    io_handle: tokio::task::JoinHandle<()>,
    /// Bulk cipher derived from the Noise handshake hash.
    /// `take_bulk_cipher()` returns `Some` exactly once. Enables
    /// bidirectional bulk transfers from the client side.
    bulk_cipher: Option<super::bulk::BulkCipher>,
}

impl BusClient {
    /// Connect to the bus server with Noise IK encrypted transport.
    ///
    /// Performs the Noise IK handshake using the provided keypair and the
    /// server's published public key. Returns a connected client ready to
    /// send requests and receive events.
    pub async fn connect(
        sender_id: Uuid,
        path: &Path,
        server_public_key: &[u8; 32],
        client_keypair: &snow::Keypair,
    ) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| IpcError::SocketBind {
                path: path.display().to_string(),
                source: e,
            })?;

        let server_creds = extract_ucred(&stream)?;
        let local_creds = PeerCredentials::local();

        let (reader, writer) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        let mut writer = tokio::io::BufWriter::new(writer);

        let mut handshake = noise::client_handshake(
            &mut reader,
            &mut writer,
            server_public_key,
            client_keypair,
            &local_creds,
            &server_creds,
        )
        .await?;

        let bulk_cipher = handshake
            .take_handshake_hash()
            .map(|h| super::bulk::kdf::derive_bulk_cipher(&h));

        let mut noise_reader = handshake.reader;
        let mut noise_writer = handshake.writer;

        let (outbound_tx, mut outbound_rx) = mpsc::channel::<bytes::Bytes>(256);
        let (inbound_tx, inbound_rx) = mpsc::channel::<bytes::Bytes>(1024);
        let (event_tx, event_rx) = mpsc::channel::<rekindle_types::subscription_events::SubscriptionEvent>(EVENT_CHANNEL_CAPACITY);
        let pending: Arc<parking_lot::Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>> =
            Arc::new(parking_lot::Mutex::new(HashMap::new()));

        let pending_clone = Arc::clone(&pending);
        let io_handle = tokio::spawn(async move {
            let mut reader = reader;
            let mut writer = writer;
            loop {
                tokio::select! {
                    result = async {
                        let mut lane_buf = [0u8; 1];
                        tokio::io::AsyncReadExt::read_exact(&mut reader, &mut lane_buf).await?;
                        if lane_buf[0] != 0x00 {
                            tracing::debug!(lane = lane_buf[0], "non-control lane from server, skipping");
                            let _ = super::framing::read_frame(&mut reader).await;
                            return Ok(None) as std::io::Result<Option<bytes::Bytes>>;
                        }
                        let payload = noise_reader.read_encrypted_frame(&mut reader).await
                            .map_err(|e| std::io::Error::other(e.to_string()))?;
                        Ok(Some(payload))
                    } => {
                        match result {
                            Ok(Some(payload)) => {
                                route_inbound(
                                    payload,
                                    &pending_clone,
                                    &inbound_tx,
                                    &event_tx,
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                                    tracing::info!("server disconnected");
                                } else {
                                    tracing::info!(error = %e, "server read error");
                                }
                                break;
                            }
                        }
                    }
                    msg = outbound_rx.recv() => {
                        if let Some(payload) = msg {
                            if tokio::io::AsyncWriteExt::write_all(&mut writer, &[0x00]).await.is_err() { break; }
                            if noise_writer.write_encrypted_frame(&mut writer, &payload).await.is_err() { break; }
                            if tokio::io::AsyncWriteExt::flush(&mut writer).await.is_err() { break; }
                        } else {
                            tracing::debug!("outbound channel closed");
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            sender_id,
            msg_ctx: MessageContext::new(sender_id),
            outbound_tx,
            inbound_rx,
            event_rx: Some(event_rx),
            pending,
            epoch: Instant::now(),
            io_handle,
            bulk_cipher,
        })
    }

    /// Connect with automatic retry and backoff.
    pub async fn connect_with_retry(
        sender_id: Uuid,
        path: &Path,
        server_public_key: &[u8; 32],
        client_keypair: &snow::Keypair,
        max_attempts: u32,
        backoff: Duration,
    ) -> Result<Self> {
        let mut last_err = None;
        for attempt in 1..=max_attempts {
            match Self::connect(sender_id, path, server_public_key, client_keypair).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "IPC connect failed, retrying");
                    last_err = Some(e);
                    if attempt < max_attempts {
                        tokio::time::sleep(backoff * attempt).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or(IpcError::ConnectionClosed))
    }

    /// Send a `Message<BusPayload>` to the bus server.
    pub async fn send(&self, msg: &Message<BusPayload>) -> Result<()> {
        let payload = encode_frame(msg)?;
        self.outbound_tx
            .send(bytes::Bytes::from(payload))
            .await
            .map_err(|_| IpcError::OutboundClosed)?;
        Ok(())
    }

    /// Send an IPC request and wait for the correlated response.
    ///
    /// Wraps the request in `BusPayload::Request`, sends it on the bus,
    /// and waits for the daemon to respond with a correlated
    /// `BusPayload::Response`. The response is decoded once in
    /// `route_inbound` and delivered as a typed `IpcResponse` via the
    /// oneshot — no re-decode needed.
    pub async fn request(
        &self,
        request: IpcRequest,
        level: SecurityLevel,
        timeout: Duration,
    ) -> Result<IpcResponse> {
        let msg = Message::new(
            &self.msg_ctx,
            BusPayload::Request(request),
            level,
            self.epoch,
        );
        let msg_id = msg.msg_id;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(msg_id, tx);

        self.send(&msg).await?;

        // [RC-1] Timeout produces typed error, not panic.
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                self.pending.lock().remove(&msg_id);
                Err(IpcError::ConnectionClosed)
            }
            Err(_) => {
                self.pending.lock().remove(&msg_id);
                Err(IpcError::RequestTimeout {
                    timeout_ms: timeout.as_secs() * 1000,
                })
            }
        }
    }

    /// Receive the next inbound frame (events, forwarded requests).
    ///
    /// Returns `None` if the server disconnected. The raw bytes can be
    /// decoded as `Message<BusPayload>` by the caller.
    pub async fn recv(&mut self) -> Option<bytes::Bytes> {
        self.inbound_rx.recv().await
    }

    /// Receive and decode the next inbound frame as a `Message<BusPayload>`.
    ///
    /// Used by the daemon subscriber to receive requests routed by the server.
    pub async fn recv_bus_message(&mut self) -> Option<Result<Message<BusPayload>>> {
        let payload: bytes::Bytes = self.inbound_rx.recv().await?;
        Some(decode_frame(&payload))
    }

    /// Send a correlated IPC response back through the bus.
    ///
    /// Used by the daemon subscriber to respond to requests. The response
    /// is pre-serialized as JSON bytes because `IpcResponse` contains
    /// `serde_json::Value` which postcard cannot handle. The server routes
    /// the response to the originating client via `correlation_id`.
    pub async fn respond(
        &self,
        response_json_bytes: Vec<u8>,
        correlation_id: Uuid,
        level: SecurityLevel,
    ) -> Result<()> {
        let msg = Message::new(
            &self.msg_ctx,
            BusPayload::Response(response_json_bytes),
            level,
            self.epoch,
        )
        .with_correlation(correlation_id);
        self.send(&msg).await
    }

    /// The client's sender ID.
    pub fn sender_id(&self) -> Uuid {
        self.sender_id
    }

    /// Take the subscription event receiver. Can only be called once.
    ///
    /// The TUI calls this at startup to get the event stream. The receiver
    /// is moved out — subsequent calls return `None`.
    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<rekindle_types::subscription_events::SubscriptionEvent>> {
        self.event_rx.take()
    }

    /// Take the bulk cipher for constructing `BulkStream` instances.
    ///
    /// Returns `Some` exactly once. The cipher is derived from the Noise
    /// handshake hash during `connect()`. Use it to create `BulkStream`s
    /// for sending bulk data from the client to the server.
    ///
    /// Returns `None` on subsequent calls or if the handshake hash was
    /// unavailable.
    pub fn take_bulk_cipher(&mut self) -> Option<super::bulk::BulkCipher> {
        self.bulk_cipher.take()
    }

    /// The client's monotonic epoch.
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// Get the current timestamp from this client's epoch.
    pub fn now(&self) -> Timestamp {
        Timestamp::now(self.epoch)
    }

    /// Create a cheaply cloneable responder handle for concurrent dispatch.
    ///
    /// The daemon subscriber uses this to spawn concurrent dispatch tasks
    /// that each need to send responses back through the bus. The responder
    /// shares the outbound channel — `mpsc::Sender` is safe to clone.
    pub fn responder(&self) -> BusResponder {
        BusResponder {
            outbound_tx: self.outbound_tx.clone(),
            msg_ctx: self.msg_ctx.clone(),
            epoch: self.epoch,
        }
    }

    /// Gracefully shut down the client, flushing pending frames.
    pub async fn shutdown(self) {
        drop(self.outbound_tx);
        let _ = self.io_handle.await;
    }
}

/// Cheaply cloneable handle for sending responses back through the IPC bus.
///
/// Used by the daemon subscriber to allow concurrent dispatch tasks to
/// each send their response independently. Shares the underlying
/// `mpsc::Sender` which is safe for concurrent use.
#[derive(Clone)]
pub struct BusResponder {
    outbound_tx: mpsc::Sender<bytes::Bytes>,
    msg_ctx: MessageContext,
    epoch: Instant,
}

impl BusResponder {
    /// Send a correlated IPC response back through the bus.
    pub async fn respond(
        &self,
        response_json_bytes: Vec<u8>,
        correlation_id: Uuid,
        level: SecurityLevel,
    ) -> Result<()> {
        let msg = Message::new(
            &self.msg_ctx,
            BusPayload::Response(response_json_bytes),
            level,
            self.epoch,
        )
        .with_correlation(correlation_id);
        let payload = encode_frame(&msg)?;
        self.outbound_tx
            .send(bytes::Bytes::from(payload))
            .await
            .map_err(|_| IpcError::OutboundClosed)?;
        Ok(())
    }
}

/// Route an inbound payload to the correct channel.
///
/// Three-way split:
/// - `BusPayload::Response` → pending oneshot waiter (request-response correlation)
/// - `BusPayload::Event(SubscriptionEvent)` → typed event channel (TUI consumes)
/// - `BusPayload::Request` → inbound channel (daemon subscriber consumes)
fn route_inbound(
    payload: bytes::Bytes,
    pending: &parking_lot::Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>,
    inbound_tx: &mpsc::Sender<bytes::Bytes>,
    event_tx: &mpsc::Sender<rekindle_types::subscription_events::SubscriptionEvent>,
) {
    let msg: Message<BusPayload> = match decode_frame(&payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "failed to decode inbound frame");
            return;
        }
    };

    match msg.payload {
        BusPayload::Response(json_bytes) => {
            let Some(corr_id) = msg.correlation_id else {
                tracing::warn!("response without correlation_id, dropping");
                return;
            };
            let response: IpcResponse = match serde_json::from_slice(&json_bytes) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "failed to deserialize IPC response from JSON");
                    return;
                }
            };
            let waiter = pending.lock().remove(&corr_id);
            if let Some(tx) = waiter {
                let _ = tx.send(response);
            } else {
                tracing::debug!(correlation_id = %corr_id, "response for unknown request");
            }
        }
        BusPayload::Event(event) => {
            // Typed subscription event — send to the dedicated event channel.
            // The TUI reads from this via DaemonClient::take_event_receiver().
            if event_tx.try_send(event).is_err() {
                tracing::debug!("event channel full or closed, event dropped");
            }
        }
        BusPayload::Request(_) => {
            // Forwarded request for daemon subscriber. CLI clients ignore these.
            if inbound_tx.try_send(payload).is_err() {
                tracing::warn!("inbound request channel full, frame dropped");
            }
        }
    }
}
