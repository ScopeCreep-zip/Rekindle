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

use tokio::net::UnixListener;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use super::error::{IpcError, Result};
use super::message::SecurityLevel;
use super::registry::ClearanceRegistry;
use super::transport::{extract_ucred, PeerCredentials};

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
    media_tx: crate::ipc::media_channel::MediaSender<Vec<u8>>,
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
    /// Agent identity and clearance registry.
    registry: RwLock<ClearanceRegistry>,
    /// Inverted-index event router for O(1) per-event subscription delivery.
    event_router: parking_lot::RwLock<crate::daemon::event_router::EventRouter>,
}

/// The IPC bus server.
pub struct BusServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<ServerState>,
    /// Noise IK static keypair for the server.
    keypair: Arc<snow::Keypair>,
}

impl BusServer {
    /// Bind the bus server to a Unix domain socket.
    ///
    /// Creates parent directory if needed. Removes stale socket before binding.
    /// Sets directory permissions to 0700 and socket to 0600. [RC-4][RC-6]
    pub fn bind(path: &Path, keypair: snow::Keypair, registry: ClearanceRegistry) -> Result<Self> {
        // Create parent directory.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IpcError::DirectoryCreate {
                path: parent.display().to_string(),
                source: e,
            })?;

            // [RC-6] Restrict parent directory to owner-only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| IpcError::DirectoryCreate {
                        path: parent.display().to_string(),
                        source: e,
                    },
                )?;
            }
        }

        // Remove stale socket. Ignore ENOENT — no TOCTOU race. [RC-4]
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: e,
                });
            }
        }

        let listener = UnixListener::bind(path).map_err(|e| IpcError::SocketBind {
            path: path.display().to_string(),
            source: e,
        })?;

        // [RC-6] Restrict socket permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: e,
                },
            )?;
        }

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
                registry: RwLock::new(registry),
                event_router: parking_lot::RwLock::new(
                    crate::daemon::event_router::EventRouter::new(),
                ),
            }),
            keypair: Arc::new(keypair),
        })
    }

    /// Run the accept loop. This future runs until cancelled.
    ///
    /// Each accepted connection spawns a per-connection task that handles
    /// the Noise handshake and bidirectional encrypted I/O.
    pub async fn run(&self) -> Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((stream, _addr)) => {
                    let peer = match extract_ucred(&stream) {
                        Ok(creds) => creds,
                        Err(e) => {
                            tracing::error!(error = %e, "rejecting: UCred extraction failed");
                            continue;
                        }
                    };

                    // [RC-6] Same-UID enforcement.
                    let my_uid = PeerCredentials::local().uid;
                    if peer.uid != my_uid {
                        tracing::error!(peer_uid = peer.uid, my_uid, "rejecting: UID mismatch");
                        continue;
                    }

                    let conn_id = self.state.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(conn_id, pid = peer.pid, "client connected");

                    let (tx, outbound_rx) = mpsc::channel::<Vec<u8>>(256);
                    let (media_tx, media_rx) =
                        crate::ipc::media_channel::media_channel::<Vec<u8>>(MEDIA_FANOUT_CAPACITY);
                    let channels = connection::ConnectionChannels {
                        tx,
                        outbound_rx,
                        media_tx,
                        media_rx,
                    };
                    let state = Arc::clone(&self.state);
                    let keypair = Arc::clone(&self.keypair);

                    tokio::spawn(async move {
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

    /// Access the registry for mutation (agent registration, rotation).
    pub async fn registry_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, ClearanceRegistry> {
        self.state.registry.write().await
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
    /// The watch receiver notifies this server when a subscription event source
    /// becomes available (on unlock) or unavailable (on lock). The delivery task
    /// subscribes to the broadcast channel and routes events through the
    /// EventRouter to subscribed connections.
    ///
    /// Re-lock safe: when the sender is replaced, the old broadcast closes,
    /// the delivery loop breaks, and the task re-awaits the next `.changed()`.
    pub fn start_event_delivery(
        &self,
        mut event_watch_rx: tokio::sync::watch::Receiver<
            Option<
                tokio::sync::broadcast::Sender<
                    rekindle_types::subscription_events::SubscriptionEvent,
                >,
            >,
        >,
    ) {
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            loop {
                // Wait for the event source to become available.
                if event_watch_rx.changed().await.is_err() {
                    // Watch sender dropped — daemon shutting down.
                    tracing::info!("event delivery: watch channel closed, exiting");
                    break;
                }

                let sender = {
                    let guard = event_watch_rx.borrow();
                    guard.clone()
                };

                let Some(sender) = sender else {
                    // Source cleared (lock transition) — loop back and wait.
                    continue;
                };

                // Subscribe and deliver until the broadcast closes.
                let mut rx = sender.subscribe();
                tracing::info!("event delivery task started");

                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let (delivered, dropped) = state.event_router.read().deliver(&event);
                            if dropped > 0 {
                                tracing::debug!(
                                    delivered,
                                    dropped,
                                    "event delivery: some recipients dropped"
                                );
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "event delivery: lagging, events dropped");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::info!("event delivery: broadcast closed (lock or shutdown)");
                            break;
                        }
                    }
                }
                // Broadcast closed — loop back to await next unlock.
            }
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
