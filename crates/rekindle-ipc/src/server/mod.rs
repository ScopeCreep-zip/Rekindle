//! IPC bus server — accepts connections, performs Noise IK handshakes,
//! dispatches daemon-bound requests, and routes agent-to-agent messages.
//!
//! The bus server lives inside the rekindle-node daemon process. It binds
//! a `UnixListener`, accepts client connections with `UCred` authentication,
//! performs encrypted handshakes, and either:
//! - Dispatches `IpcRequest` frames to `daemon::dispatch` and returns the
//!   `IpcResponse` to the originating connection (request-response pattern)
//! - Routes non-IpcRequest frames between connected agents (pub-sub pattern)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use super::error::Result;
use super::message::SecurityLevel;
use super::registry::ClearanceRegistry;
use super::transport::{Listener, PeerCredentials};

mod connection;
mod routing;

/// Rate limit: max requests per second per connection.
const RATE_LIMIT_MAX_TOKENS: u32 = 100;
/// Rate limit: refill interval in milliseconds.
const RATE_LIMIT_REFILL_MS: u64 = 1000;

/// Per-connection media fan-out queue depth.
///
/// Bounds how many undelivered video frames the server buffers for one slow
/// client before evicting the oldest (drop-oldest). ~4 s of a single 30 fps
/// stream — enough to ride out a socket-write hiccup, small enough that a
/// stalled client never accrues stale video. A late frame is worthless.
const MEDIA_FANOUT_CAPACITY: usize = 128;

/// Per-connection state tracked by the bus server.
struct ConnectionState {
    /// Agent identity (set on first message, immutable thereafter).
    agent_id: Option<Uuid>,
    /// Registry-verified agent name from Noise IK handshake.
    verified_name: Option<String>,
    /// Outbound channel to this connection's I/O task (responses + events).
    tx: mpsc::Sender<Vec<u8>>,
    /// Bounded, drop-oldest media fan-out queue to this connection's I/O task.
    ///
    /// Separate from `tx` so a burst of video frames can never evict a queued
    /// response or event, and so the media path can drop the OLDEST frame on
    /// overflow (which `tx`, a plain mpsc, cannot). Carries encoded
    /// `Message<BusPayload::Media>` frames.
    media_tx: crate::media_channel::MediaSender<Vec<u8>>,
    /// The client's Noise static public key, proven by the handshake.
    static_key: [u8; 32],
    /// Peer OS-level credentials.
    peer: PeerCredentials,
    /// Security clearance level from registry lookup.
    security_clearance: SecurityLevel,
    /// When this connection was established.
    connected_at: Instant,
    /// Token bucket rate limiter: tokens remaining this window.
    rate_tokens: std::sync::atomic::AtomicU32,
    /// Epoch ms when tokens were last refilled.
    last_token_refill: std::sync::atomic::AtomicU64,
}

/// Shared state for the bus server, accessible from per-connection tasks.
struct ServerState {
    /// Active connections.
    connections: RwLock<HashMap<u64, ConnectionState>>,
    /// Request-response routing: msg_id → originating connection_id.
    pending_requests: RwLock<HashMap<Uuid, u64>>,
    /// Name-based unicast: verified_name → connection_id.
    name_to_conn: RwLock<HashMap<String, u64>>,
    /// Connection ID generator.
    next_conn_id: AtomicU64,
    /// Monotonic epoch for timestamps.
    epoch: Instant,
    /// The bus path, bound into every connection's Noise prologue.
    socket_path: PathBuf,
    /// Agent identity and clearance registry — the one the host's
    /// dispatch registers agents into.
    registry: Arc<RwLock<ClearanceRegistry>>,
    /// Inverted-index event router for O(1) per-event subscription delivery.
    event_router: parking_lot::RwLock<crate::event_router::EventRouter>,
    /// Woken whenever a connection finishes its cleanup.
    connection_closed: tokio::sync::Notify,
}

/// The IPC bus server.
pub struct BusServer {
    listener: Listener,
    socket_path: PathBuf,
    state: Arc<ServerState>,
    /// Noise IK static keypair for the server.
    keypair: Arc<snow::Keypair>,
    /// The per-connection tasks, so shutdown can wait for them.
    connections: tokio_util::task::TaskTracker,
    /// Stops the accept loop and the event delivery task.
    stopping: tokio_util::sync::CancellationToken,
}

impl BusServer {
    /// Bind the bus server at `path` (`transport::Listener::bind`: an
    /// owner-only Unix socket, or an owner-only named pipe on Windows).
    pub fn bind(
        path: &Path,
        keypair: snow::Keypair,
        registry: Arc<RwLock<ClearanceRegistry>>,
    ) -> Result<Self> {
        let listener = Listener::bind(path)?;

        tracing::info!(path = %path.display(), "IPC bus server bound");

        Ok(Self {
            listener,
            socket_path: path.to_owned(),
            state: Arc::new(ServerState {
                connections: RwLock::new(HashMap::new()),
                pending_requests: RwLock::new(HashMap::new()),
                name_to_conn: RwLock::new(HashMap::new()),
                next_conn_id: AtomicU64::new(1),
                epoch: Instant::now(),
                socket_path: path.to_owned(),
                registry,
                event_router: parking_lot::RwLock::new(crate::event_router::EventRouter::new()),
                connection_closed: tokio::sync::Notify::new(),
            }),
            keypair: Arc::new(keypair),
            connections: tokio_util::task::TaskTracker::new(),
            stopping: tokio_util::sync::CancellationToken::new(),
        })
    }

    /// Run the accept loop until [`Self::shutdown`] (or until cancelled).
    ///
    /// Each accepted connection spawns a per-connection task that handles
    /// the Noise handshake and bidirectional encrypted I/O.
    pub async fn run(&self) -> Result<()> {
        loop {
            let accepted = tokio::select! {
                () = self.stopping.cancelled() => return Ok(()),
                accepted = self.listener.accept() => accepted,
            };
            match accepted {
                // Refused by the platform's peer check; already logged.
                Ok(None) => {}
                Ok(Some((stream, peer))) => {
                    let conn_id = self.state.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(conn_id, pid = ?peer.pid, "client connected");

                    let (tx, outbound_rx) = mpsc::channel::<Vec<u8>>(256);
                    let (media_tx, media_rx) =
                        crate::media_channel::media_channel::<Vec<u8>>(MEDIA_FANOUT_CAPACITY);
                    let channels = connection::ConnectionChannels {
                        tx,
                        outbound_rx,
                        media_tx,
                        media_rx,
                    };
                    let state = Arc::clone(&self.state);
                    let keypair = Arc::clone(&self.keypair);

                    self.connections.spawn(async move {
                        connection::handle_connection(
                            state, conn_id, stream, channels, peer, keypair,
                        )
                        .await;
                    });
                }
                Err(e) => {
                    // [RC-1] Log actual OS error, don't conflate.
                    tracing::error!(error = %e, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Drain the bus: stop accepting, wait for the daemon's own connection
    /// to close (its subscriber has then sent every response it owes),
    /// then close each client connection once its queued frames are
    /// written. Whatever is still open at `deadline` is left to process
    /// exit.
    pub async fn shutdown(&self, deadline: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + deadline;
        self.stopping.cancel();
        self.connections.close();

        // The daemon's connection closes once its subscriber has drained.
        loop {
            let mut closed = std::pin::pin!(self.state.connection_closed.notified());
            closed.as_mut().enable();
            if !self
                .state
                .name_to_conn
                .read()
                .await
                .contains_key(DAEMON_AGENT_NAME)
            {
                break;
            }
            if tokio::time::timeout_at(deadline, closed).await.is_err() {
                tracing::warn!("daemon connection still open at the drain deadline");
                break;
            }
        }

        // Dropping a connection's senders ends its I/O loop once the queue
        // it already holds is written.
        let closing: Vec<u64> = self
            .state
            .connections
            .read()
            .await
            .keys()
            .copied()
            .collect();
        for conn_id in &closing {
            self.state.connections.write().await.remove(conn_id);
            self.state.event_router.write().remove_connection(*conn_id);
        }
        if tokio::time::timeout_at(deadline, self.connections.wait())
            .await
            .is_err()
        {
            tracing::warn!(
                open = self.connections.len(),
                "connections still open at the drain deadline"
            );
        }
    }

    /// Number of active connections.
    pub async fn connection_count(&self) -> usize {
        self.state.connections.read().await.len()
    }

    /// The server's monotonic epoch.
    pub fn epoch(&self) -> Instant {
        self.state.epoch
    }

    /// The socket path this server is bound to.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start the event delivery system using a watch channel.
    ///
    /// The watch carries the subscription event source while the daemon is
    /// unlocked. The delivery task follows it across lock cycles
    /// ([`crate::event_source::follow`]) and routes each event through the
    /// EventRouter to subscribed connections, until the server shuts down.
    pub fn start_event_delivery(&self, source: crate::event_source::EventSource) {
        let state = Arc::clone(&self.state);
        let stopping = self.stopping.clone();
        self.connections.spawn(async move {
            crate::event_source::follow(&stopping, source, |event| {
                let (delivered, dropped) = state.event_router.read().deliver(&event);
                if dropped > 0 {
                    tracing::debug!(
                        delivered,
                        dropped,
                        "event delivery: some recipients dropped"
                    );
                }
                std::future::ready(())
            })
            .await;
            tracing::info!("event delivery stopped");
        });
    }
}

impl Drop for BusServer {
    fn drop(&mut self) {
        // Clean up socket file on shutdown.
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(path = %self.socket_path.display(), "IPC socket removed");
    }
}

/// Well-known agent name for the daemon's internal bus subscriber.
///
/// The daemon registers with this name when it connects to its own socket.
/// Requests are unicast to this connection. Other agents MUST NOT register
/// with this name — the registry enforces uniqueness.
pub const DAEMON_AGENT_NAME: &str = "daemon";
