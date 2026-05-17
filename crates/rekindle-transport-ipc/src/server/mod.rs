//! IPC bus server: accepts connections, Noise handshakes, frame dispatch.
//!
//! Generic over `FrameRouter` — the application injects its routing logic
//! via this trait. The transport never inspects payload contents.
//!
//! Transport-level concerns (ack, heartbeat, bulk ack/nack, shutdown) are
//! handled internally via tag-byte multiplexing. Application frames
//! (tag >= 0x80) are delivered to `FrameRouter::route_frame` with the
//! tag byte stripped.

pub mod connection;
pub mod state;
pub mod write_loop;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use std::os::unix::net::UnixListener as StdUnixListener;

use bytes::Bytes;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::backpressure::GlobalMemoryGuard;
use crate::bulk;
use crate::config::IpcConfig;
use crate::envelope::SharedFrame;
use crate::error::{IpcError, IpcResult};
use crate::socket::{extract_ucred, PeerCredentials};
use crate::transport_frame::ConnectionPhase;

use state::{PendingRequests, ServerState};

/// Application-level frame routing callback.
///
/// Implementations MUST be `Send + Sync` and non-blocking.
/// A blocking `route_frame` starves the connection's read loop
/// and triggers ack timeouts on the sender.
///
/// Bulk transfers are delivered via streaming: `on_bulk_chunk` is called
/// for each decrypted, reassembled chunk as it arrives. The caller decides
/// whether to buffer, write to disk, or forward. Memory usage is
/// O(chunk_size) regardless of total transfer size.
///
/// `on_bulk_complete` fires after the final chunk's Merkle root is verified.
/// It carries metadata only — no payload. The BulkAck is sent to the peer
/// after this callback returns.
pub trait FrameRouter: Send + Sync + 'static {
    /// Route a decrypted application frame from a connection.
    /// `payload` has the tag byte stripped — pure application content.
    fn route_frame(&self, state: &ServerState, conn_id: u64, payload: Bytes);

    /// A decrypted, reassembled bulk data chunk, delivered in order.
    ///
    /// Called once per DATA chunk as it arrives. `data` is the decrypted
    /// plaintext (~65KB max). The chunk has been AEAD-verified and
    /// digest-computed. BulkFin frames are NOT delivered here — the
    /// completion signal is `on_bulk_complete`.
    ///
    /// The caller owns the processing — write to disk, hash, forward, or
    /// accumulate into a Vec. The transport does not buffer.
    ///
    /// MUST NOT block. Blocking stalls the connection's control loop and
    /// deadlocks the decrypt pool. Callers that need blocking I/O must
    /// use a channel or `tokio::spawn` inside this callback.
    fn on_bulk_chunk(
        &self, state: &ServerState,
        conn_id: u64, stream_id: u8, chunk_seq: u32,
        data: &[u8],
    );

    /// A bulk transfer completed successfully. Merkle root verified.
    ///
    /// This is a metadata notification — no payload. The chunks were already
    /// delivered via `on_bulk_chunk`. The BulkAck is sent to the peer after
    /// this callback returns.
    fn on_bulk_complete(
        &self, state: &ServerState,
        conn_id: u64, stream_id: u8, total_bytes: u64, total_chunks: u64,
    );

    /// Connection lifecycle state changed.
    /// Called on every phase transition for observability.
    fn on_connection_state_changed(
        &self, state: &ServerState,
        conn_id: u64, old: ConnectionPhase, new: ConnectionPhase,
    );
}

/// The IPC bus server. Generic over the application's frame router.
pub struct IpcServer<R: FrameRouter> {
    listener: StdUnixListener,
    socket_path: PathBuf,
    state: Arc<ServerState>,
    keypair: Arc<snow::Keypair>,
    router: Arc<R>,
    config: Arc<IpcConfig>,
    encrypt_pool: Arc<rayon::ThreadPool>,
    decrypt_pool: Arc<rayon::ThreadPool>,
    buffer_pool: Arc<bulk::BufferPool>,
    bulk_counters: Arc<bulk::BulkCounters>,
    /// Semaphore enforcing max_connections. Acquired before spawn,
    /// OwnedPermit moved into the connection task and dropped on exit.
    conn_semaphore: Arc<Semaphore>,
    /// Cancellation token for graceful shutdown. Cancelled in Drop,
    /// checked by every connection handler's control loop via select!.
    cancel_token: CancellationToken,
    /// Global memory guard for in-flight bulk data backpressure.
    memory_guard: Arc<GlobalMemoryGuard>,
}

impl<R: FrameRouter> IpcServer<R> {
    /// Bind the server to a Unix domain socket.
    ///
    /// Creates parent directory (0700), removes stale socket, binds,
    /// sets socket permissions (0600).
    pub fn bind(
        path: &Path,
        keypair: snow::Keypair,
        router: R,
        config: IpcConfig,
    ) -> IpcResult<Self> {
        config
            .validate()
            .map_err(|e| IpcError::HandshakeFailed { reason: e })?;

        // Reject empty path before any filesystem operations.
        if path.as_os_str().is_empty() {
            return Err(IpcError::SocketBind {
                path: "(empty)".into(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "socket path must not be empty",
                ),
            });
        }

        if let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() {
                // path is just a filename with no directory component — bind in CWD
            } else {
                std::fs::create_dir_all(parent).map_err(|e| IpcError::DirectoryCreate {
                    path: parent.display().to_string(),
                    source: e,
                })?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let meta = std::fs::metadata(parent).map_err(|e| IpcError::DirectoryCreate {
                        path: parent.display().to_string(),
                        source: e,
                    })?;
                    let mode = meta.permissions().mode();
                    let uid = rustix::process::getuid().as_raw();
                    let gid = rustix::process::getgid().as_raw();
                    let file_uid = rustix::fs::stat(parent)
                        .map(|s| s.st_uid)
                        .unwrap_or(u32::MAX);
                    let file_gid = rustix::fs::stat(parent)
                        .map(|s| s.st_gid)
                        .unwrap_or(u32::MAX);

                    let writable = if uid == file_uid {
                        mode & 0o200 != 0
                    } else if gid == file_gid {
                        mode & 0o020 != 0
                    } else {
                        mode & 0o002 != 0
                    };

                    if !writable {
                        return Err(IpcError::SocketBind {
                            path: path.display().to_string(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                format!("parent directory {} is not writable (mode {:04o})",
                                    parent.display(), mode & 0o777),
                            ),
                        });
                    }

                    let _ = std::fs::set_permissions(
                        parent,
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
        }

        let _ = std::fs::remove_file(path);

        let std_listener = StdUnixListener::bind(path).map_err(|e| IpcError::SocketBind {
            path: path.display().to_string(),
            source: e,
        })?;
        // Apply listen_backlog from config. StdUnixListener::bind() calls
        // listen(fd, 128) internally. We override with the configured value
        // via rustix::net::listen, which calls listen(2) directly.
        // The kernel clamps to net.core.somaxconn (default 4096 since 5.4).
        // Apply listen_backlog from config. StdUnixListener::bind() calls
        // listen(fd, 128) internally. We override with the configured value.
        // The kernel clamps to net.core.somaxconn (default 4096 since 5.4).
        #[cfg(unix)]
        {
            use std::os::fd::AsFd;
            rustix::net::listen(std_listener.as_fd(), config.listen_backlog.min(i32::MAX as u32) as i32)
                .map_err(|e| IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: std::io::Error::from(e),
                })?;
        }
        std_listener.set_nonblocking(true).map_err(|e| IpcError::SocketBind {
            path: path.display().to_string(),
            source: e,
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }

        tracing::info!(path = %path.display(), "IPC server bound");

        let encrypt_pool = bulk::encrypt::build_encrypt_pool(config.encrypt_workers);
        let decrypt_pool = bulk::encrypt::build_encrypt_pool(config.encrypt_workers);
        let buffer_pool = bulk::BufferPool::new(config.pool_slab_count);
        let conn_semaphore = Arc::new(Semaphore::new(config.max_connections as usize));
        let memory_guard = Arc::new(GlobalMemoryGuard::new(config.global_memory_limit));

        Ok(Self {
            listener: std_listener,
            socket_path: path.to_owned(),
            state: Arc::new(ServerState {
                connections: dashmap::DashMap::new(),
                pending_requests: parking_lot::Mutex::new(PendingRequests::new()),
                name_to_conn: parking_lot::RwLock::new(HashMap::new()),
                next_conn_id: AtomicU64::new(1),
                epoch: Instant::now(),
                global_rate_limiter: state::TokenBucket::new(config.rate_limit_global_per_sec, 1000),
            }),
            keypair: Arc::new(keypair),
            router: Arc::new(router),
            config: Arc::new(config),
            encrypt_pool,
            decrypt_pool,
            buffer_pool,
            bulk_counters: bulk::BulkCounters::new(),
            conn_semaphore,
            cancel_token: CancellationToken::new(),
            memory_guard,
        })
    }

    /// Run the accept loop. Runs until cancelled.
    pub async fn run(&self) -> IpcResult<()> {
        let listener = UnixListener::from_std(self.listener.try_clone().map_err(|e| {
            IpcError::SocketBind {
                path: self.socket_path.display().to_string(),
                source: e,
            }
        })?).map_err(|e| IpcError::SocketBind {
            path: self.socket_path.display().to_string(),
            source: e,
        })?;

        loop {
            // Acquire a connection permit BEFORE accept. This blocks when
            // max_connections is reached. The permit is moved into the
            // connection handler task and released when the task exits.
            let permit = Arc::clone(&self.conn_semaphore)
                .acquire_owned()
                .await
                .map_err(|_| IpcError::ConnectionClosed)?;

            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "accept failed");
                    drop(permit); // release the slot
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            };

            let peer = match extract_ucred(&stream) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "UCred extraction failed");
                    continue; // permit dropped — slot released
                }
            };

            let my_uid = PeerCredentials::local().uid;
            if peer.uid != my_uid {
                tracing::error!(peer_uid = peer.uid, my_uid, "UID mismatch");
                continue; // permit dropped — slot released
            }

            let conn_id = self.state.next_conn_id.fetch_add(1, Ordering::Relaxed);
            tracing::info!(conn_id, pid = peer.pid, "client connected");

            let (resp_tx, resp_rx) = mpsc::channel::<Bytes>(16);
            let (event_tx, event_rx) = mpsc::channel::<SharedFrame>(32);

            let resources = connection::ConnResources {
                state: Arc::clone(&self.state),
                conn_id,
                stream,
                resp_tx,
                event_tx,
                resp_rx,
                event_rx,
                peer,
                keypair: Arc::clone(&self.keypair),
                router: Arc::clone(&self.router),
                config: Arc::clone(&self.config),
                encrypt_pool: Arc::clone(&self.encrypt_pool),
                decrypt_pool: Arc::clone(&self.decrypt_pool),
                buffer_pool: Arc::clone(&self.buffer_pool),
                bulk_counters: Arc::clone(&self.bulk_counters),
                cancel_token: self.cancel_token.child_token(),
                memory_guard: Arc::clone(&self.memory_guard),
            };

            tokio::spawn(async move {
                connection::handle_connection(resources).await;
                // OwnedSemaphorePermit dropped here — releases the connection slot.
                drop(permit);
            });
        }
    }

    pub fn state(&self) -> Arc<ServerState> {
        Arc::clone(&self.state)
    }

    pub fn connection_count(&self) -> usize {
        self.state.connections.len()
    }

    pub fn bulk_counters(&self) -> &Arc<bulk::BulkCounters> {
        &self.bulk_counters
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn memory_guard(&self) -> &Arc<GlobalMemoryGuard> {
        &self.memory_guard
    }
}

impl<R: FrameRouter> Drop for IpcServer<R> {
    fn drop(&mut self) {
        // Signal all connection handlers to shut down.
        self.cancel_token.cancel();
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(path = %self.socket_path.display(), "IPC socket removed");
    }
}
