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
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;

use rekindle_types::video::MediaFrame;

use super::error::{IpcError, Result};
use super::framing::{decode_frame, encode_frame};
use super::media_channel::{media_channel, MediaReceiver, MediaSender};
use super::message::{Message, MessageContext, SecurityLevel, Timestamp};
use super::noise;
use super::protocol::{BusPayload, IpcRequest, IpcResponse};
use super::transport::{extract_ucred, PeerCredentials};

/// Bounded depth of the client-side media frame queue.
///
/// Video frames arrive at frame rate and a late frame is worthless, so the
/// queue drops the OLDEST frame on overflow — it never grows unboundedly or
/// blocks the I/O loop. ~4 s of one 30 fps stream: enough to absorb a slow
/// consumer without hoarding stale video.
const MEDIA_RECEIVER_CAPACITY: usize = 128;

/// The IPC bus client used by frontends and agents.
pub struct BusClient {
    sender_id: Uuid,
    msg_ctx: MessageContext,
    /// Outbound frames to send to the server.
    outbound_tx: mpsc::Sender<Vec<u8>>,
    /// Inbound frames received from the server (forwarded requests for daemon subscriber).
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    /// Typed subscription events from the server's EventRouter.
    /// Taken once by the consumer via `take_event_receiver()`.
    event_rx:
        Option<mpsc::UnboundedReceiver<rekindle_types::subscription_events::SubscriptionEvent>>,
    /// Bounded, drop-oldest media frame channel. Taken once via
    /// `take_media_receiver()`. Video frames arrive at frame rate and a late
    /// one is worthless, so this bypasses the ordered event channel — a full
    /// queue drops the oldest frame, never blocking the I/O loop.
    media_rx: Option<MediaReceiver<MediaFrame>>,
    /// Pending request-response waiters, keyed by msg_id.
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>>,
    /// Monotonic epoch for timestamp generation.
    epoch: Instant,
    /// Handle to the multiplexed I/O task.
    io_handle: tokio::task::JoinHandle<()>,
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

        let transport = noise::client_handshake(
            &mut reader,
            &mut writer,
            server_public_key,
            client_keypair,
            &local_creds,
            &server_creds,
        )
        .await?;

        let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(256);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Vec<u8>>(1024);
        let (event_tx, event_rx) =
            mpsc::unbounded_channel::<rekindle_types::subscription_events::SubscriptionEvent>();
        let (media_tx, media_rx) = media_channel::<MediaFrame>(MEDIA_RECEIVER_CAPACITY);
        let pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let pending_clone = Arc::clone(&pending);
        let io_handle = tokio::spawn(async move {
            let mut transport = transport;
            let mut reader = reader;
            let mut writer = writer;
            loop {
                tokio::select! {
                    result = transport.read_encrypted_frame(&mut reader) => {
                        if let Ok(payload) = result {
                            route_inbound(
                                payload,
                                &pending_clone,
                                &inbound_tx,
                                &event_tx,
                                &media_tx,
                            ).await;
                        } else {
                            tracing::info!("server disconnected");
                            break;
                        }
                    }
                    msg = outbound_rx.recv() => {
                        if let Some(mut payload) = msg {
                            let result = transport
                                .write_encrypted_frame(&mut writer, &payload)
                                .await;
                            zeroize::Zeroize::zeroize(&mut payload);
                            if let Err(e) = result {
                                tracing::debug!(error = %e, "write failed, closing");
                                break;
                            }
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
            media_rx: Some(media_rx),
            pending,
            epoch: Instant::now(),
            io_handle,
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
            .send(payload)
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
        self.pending.lock().await.insert(msg_id, tx);

        self.send(&msg).await?;

        // [RC-1] Timeout produces typed error, not panic.
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&msg_id);
                Err(IpcError::ConnectionClosed)
            }
            Err(_) => {
                self.pending.lock().await.remove(&msg_id);
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
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.inbound_rx.recv().await
    }

    /// Receive and decode the next inbound frame as a `Message<BusPayload>`.
    ///
    /// Used by the daemon subscriber to receive requests routed by the server.
    pub async fn recv_bus_message(&mut self) -> Option<Result<Message<BusPayload>>> {
        let payload = self.inbound_rx.recv().await?;
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

    /// Push a compressed video frame to subscribed clients (daemon → client).
    ///
    /// This is the daemon's media send entrypoint — the analog of [`respond`]
    /// for the frame path. It wraps the frame in `BusPayload::Media` (no
    /// correlation id: it is an unsolicited push, not a reply) and sends it on
    /// the bus. The server's router fans it out to every subscribed
    /// connection's bounded, drop-oldest media queue, bypassing the ordered
    /// event stream entirely.
    ///
    /// Only the daemon agent's frames are fanned out; a frame from any other
    /// connection is dropped by the server. This is the API the future
    /// capture/encode wiring calls once a frame is compressed.
    ///
    /// [`respond`]: Self::respond
    pub async fn send_media(&self, frame: MediaFrame, level: SecurityLevel) -> Result<()> {
        let msg = Message::new(&self.msg_ctx, BusPayload::Media(frame), level, self.epoch);
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
    pub fn take_event_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<rekindle_types::subscription_events::SubscriptionEvent>>
    {
        self.event_rx.take()
    }

    /// Take the media frame receiver. Can only be called once.
    ///
    /// The video consumer (TUI, headless recorder, `ffplay` pipe, …) calls
    /// this to get the frame stream. The receiver is bounded and drop-oldest:
    /// a late frame is dropped, never queued unboundedly, because a stale
    /// video frame is worthless. Mirrors [`take_event_receiver`], but the
    /// media queue is bounded where the event queue is not.
    ///
    /// [`take_event_receiver`]: Self::take_event_receiver
    pub fn take_media_receiver(&mut self) -> Option<MediaReceiver<MediaFrame>> {
        self.media_rx.take()
    }

    /// The client's monotonic epoch.
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// Get the current timestamp from this client's epoch.
    pub fn now(&self) -> Timestamp {
        Timestamp::now(self.epoch)
    }

    /// Gracefully shut down the client, flushing pending frames.
    pub async fn shutdown(self) {
        drop(self.outbound_tx);
        let _ = self.io_handle.await;
    }
}

/// Route an inbound payload to the correct channel.
///
/// Four-way split:
/// - `BusPayload::Response` → pending oneshot waiter (request-response correlation)
/// - `BusPayload::Event(SubscriptionEvent)` → typed event channel (TUI consumes)
/// - `BusPayload::Media(MediaFrame)` → bounded drop-oldest media channel (video consumer)
/// - `BusPayload::Request` → inbound channel (daemon subscriber consumes)
async fn route_inbound(
    payload: Vec<u8>,
    pending: &Mutex<HashMap<Uuid, oneshot::Sender<IpcResponse>>>,
    inbound_tx: &mpsc::Sender<Vec<u8>>,
    event_tx: &mpsc::UnboundedSender<rekindle_types::subscription_events::SubscriptionEvent>,
    media_tx: &MediaSender<MediaFrame>,
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
            let waiter = pending.lock().await.remove(&corr_id);
            if let Some(tx) = waiter {
                let _ = tx.send(response);
            } else {
                tracing::debug!(correlation_id = %corr_id, "response for unknown request");
            }
        }
        BusPayload::Event(event) => {
            // Typed subscription event — send to the dedicated event channel.
            // The TUI reads from this via DaemonClient::take_event_receiver().
            if event_tx.send(event).is_err() {
                tracing::debug!("event channel closed, event dropped");
            }
        }
        BusPayload::Media(frame) => {
            // Compressed video frame — hand to the bounded, drop-oldest media
            // channel the consumer took via take_media_receiver(). The queue
            // never blocks the I/O loop: on overflow it drops the OLDEST frame,
            // because a late video frame is worthless. If the consumer never
            // took the receiver, frames simply accumulate to the cap and evict.
            let evicted = media_tx.send(frame);
            if evicted {
                tracing::trace!("media queue full, dropped oldest frame");
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
