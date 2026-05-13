//! IPC bus server — accepts connections, performs Noise IK handshakes,
//! dispatches daemon-bound requests, and routes agent-to-agent messages.
//!
//! # Module layout
//!
//! - `state`       — Shared data structures (ConnectionState, PendingRequests, TokenBucket)
//! - `connection`  — Per-connection lifecycle (handshake, lane-aware I/O loop, cleanup)
//! - `routing`     — Control-plane frame routing (classify, forward, subscribe handling)
//! - `lane`        — Lane byte protocol (multiplex control + bulk on one socket)
//! - `constants`   — Server-wide constants (DAEMON_AGENT_NAME, rate limits)
//!
//! # Lock architecture (100K-agent optimized)
//!
//! - `connections`: `DashMap<u64, ConnectionState>` — sharded by conn_id
//! - `pending_requests`: `parking_lot::Mutex<PendingRequests>` — dual-indexed
//! - `name_to_conn`: `parking_lot::RwLock<HashMap>` — low cardinality
//! - `registry`: `parking_lot::RwLock<ClearanceRegistry>`
//! - `event_router`: `Arc<ShardedEventRouter>` — internal striped locking

mod connection;
pub mod constants;
pub mod lane;
mod routing;
pub mod state;
mod write_loop;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use super::error::{IpcError, Result};
use super::event_router::ShardedEventRouter;
use super::message::SharedFrame;
use super::registry::ClearanceRegistry;
use super::transport::{extract_ucred, PeerCredentials};

use state::{PendingRequests, ServerState};

pub use constants::DAEMON_AGENT_NAME;

/// The IPC bus server.
pub struct BusServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<ServerState>,
    keypair: Arc<snow::Keypair>,
    /// Shared rayon pool for bulk encryption/decryption across all connections.
    encrypt_pool: Arc<rayon::ThreadPool>,
    /// Shared buffer pool for zero-allocation bulk frame I/O.
    buffer_pool: Arc<super::bulk::BufferPool>,
    /// Shared atomic counters for bulk transfer observability.
    bulk_counters: Arc<super::bulk::BulkCounters>,
    /// Shared event journal for cursor-based resumption.
    event_journal: Arc<super::journal::EventJournal>,
}

impl BusServer {
    /// Bind the bus server to a Unix domain socket.
    pub fn bind(
        path: &Path,
        keypair: snow::Keypair,
        registry: ClearanceRegistry,
        encrypt_pool: Arc<rayon::ThreadPool>,
        buffer_pool: Arc<super::bulk::BufferPool>,
        bulk_counters: Arc<super::bulk::BulkCounters>,
        event_journal: Arc<super::journal::EventJournal>,
    ) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IpcError::DirectoryCreate {
                path: parent.display().to_string(),
                source: e,
            })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| IpcError::DirectoryCreate {
                        path: parent.display().to_string(),
                        source: e,
                    })?;
            }
        }

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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: e,
                })?;
        }

        tracing::info!(path = %path.display(), "IPC bus server bound");

        Ok(Self {
            listener,
            socket_path: path.to_owned(),
            state: Arc::new(ServerState {
                connections: dashmap::DashMap::new(),
                pending_requests: parking_lot::Mutex::new(PendingRequests::new()),
                name_to_conn: parking_lot::RwLock::new(HashMap::new()),
                next_conn_id: AtomicU64::new(1),
                epoch: Instant::now(),
                registry: parking_lot::RwLock::new(registry),
                event_router: Arc::new(ShardedEventRouter::new()),
                daemon_tx: parking_lot::RwLock::new(None),
            }),
            keypair: Arc::new(keypair),
            encrypt_pool,
            buffer_pool,
            bulk_counters,
            event_journal,
        })
    }

    /// Run the accept loop. This future runs until cancelled.
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

                    let my_uid = PeerCredentials::local().uid;
                    if peer.uid != my_uid {
                        tracing::error!(peer_uid = peer.uid, my_uid, "rejecting: UID mismatch");
                        continue;
                    }

                    let conn_id = self.state.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(conn_id, pid = peer.pid, "client connected");

                    let (response_tx, response_rx) = mpsc::channel::<Bytes>(64);
                    let (event_tx, event_rx) = mpsc::channel::<SharedFrame>(256);
                    let state = Arc::clone(&self.state);
                    let keypair = Arc::clone(&self.keypair);
                    let enc_pool = Arc::clone(&self.encrypt_pool);
                    let buf_pool = Arc::clone(&self.buffer_pool);
                    let counters = Arc::clone(&self.bulk_counters);

                    tokio::spawn(async move {
                        connection::handle_connection(
                            state, conn_id, stream,
                            response_tx, event_tx,
                            response_rx, event_rx,
                            peer, keypair,
                            enc_pool, buf_pool, counters,
                        ).await;
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Access the event router (for event delivery setup).
    pub fn event_router(&self) -> &Arc<ShardedEventRouter> {
        &self.state.event_router
    }

    /// Register the daemon subscriber's RoutedFrame channel.
    pub fn register_daemon_channel(&self, tx: mpsc::Sender<super::message::RoutedFrame>) {
        *self.state.daemon_tx.write() = Some(tx);
    }

    /// Number of active connections.
    pub fn connection_count(&self) -> usize {
        self.state.connections.len()
    }

    /// The server's monotonic epoch.
    pub fn epoch(&self) -> Instant {
        self.state.epoch
    }

    /// The socket path this server is bound to.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Access the shared server state.
    pub fn state(&self) -> Arc<ServerState> {
        Arc::clone(&self.state)
    }

    /// Start the event delivery system using a watch channel.
    pub fn start_event_delivery(
        &self,
        mut event_watch_rx: tokio::sync::watch::Receiver<
            Option<tokio::sync::broadcast::Sender<rekindle_types::subscription_events::SubscriptionEvent>>,
        >,
    ) {
        let router = Arc::clone(&self.state.event_router);
        let journal = Arc::clone(&self.event_journal);
        tokio::spawn(async move {
            loop {
                if event_watch_rx.changed().await.is_err() {
                    tracing::info!("event delivery: watch channel closed, exiting");
                    break;
                }

                let sender = {
                    let guard = event_watch_rx.borrow();
                    guard.clone()
                };

                let Some(sender) = sender else { continue };

                let mut rx = sender.subscribe();
                tracing::info!("event delivery task started");

                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            journal.append(event.clone());
                            let (delivered, dropped) = router.deliver(&event);
                            if dropped > 0 {
                                tracing::debug!(delivered, dropped, "event delivery: drops");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "event delivery: lagging");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::info!("event delivery: broadcast closed");
                            break;
                        }
                    }
                }
            }
        });
    }
}

impl Drop for BusServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(path = %self.socket_path.display(), "IPC socket removed");
    }
}
