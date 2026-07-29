//! IPC bus server — accepts connections, authenticates via UCred + Noise IK,
//! spawns per-connection lane tasks via `io::connection`, and delivers
//! frames to the application through the FrameRouter trait.

pub mod handle;
pub mod arena;

pub use handle::ConnectionHandle;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::net::UnixListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::v4::config::{SessionConfig, ServerConfig};
use crate::v4::io::connection::{self, ConnectionParams};
use crate::v4::io::lane_channels::{RecvBufPool, WireBufPool};
use crate::v4::router::FrameRouter;
use crate::v4::session::handshake::{listener_handshake, HandshakeConfig};
use crate::v4::session::SessionRole;
use crate::v4::socket::{self, PeerCredentials};

/// The IPC bus server.
pub struct IpcServer<F, R>
where
    F: Fn(ConnectionHandle) -> R + Send + Sync + 'static,
    R: FrameRouter,
{
    listener: UnixListener,
    socket_path: PathBuf,
    router_factory: Arc<F>,
    config: SessionConfig,
    handshake_config: HandshakeConfig,
    next_conn_id: AtomicU64,
    keypair: snow::Keypair,
    conn_semaphore: Arc<Semaphore>,
    cancel_token: CancellationToken,
    encrypt_pool: Arc<rayon::ThreadPool>,
    wire_pool: WireBufPool,
    recv_pool: RecvBufPool,
    counters: Arc<crate::v4::bulk::counters::BulkCounters>,
    _marker: std::marker::PhantomData<R>,
}

#[derive(Debug)]
pub enum ServerError {
    Bind { path: String, source: std::io::Error },
    Accept(std::io::Error),
}

impl<F, R> IpcServer<F, R>
where
    F: Fn(ConnectionHandle) -> R + Send + Sync + 'static,
    R: FrameRouter,
{
    pub async fn bind(
        path: &Path,
        keypair: snow::Keypair,
        router_factory: F,
        server_config: ServerConfig,
    ) -> Result<Self, ServerError> {
        let config = server_config.session;
        let handshake_config = server_config.handshake;
        let counters = server_config.counters;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| ServerError::Bind {
                    path: path.display().to_string(), source: e,
                })?;
            }
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).map_err(|e| ServerError::Bind {
            path: path.display().to_string(), source: e,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        tracing::info!(path = %path.display(), "IPC server bound");

        let max_connections = config.max_connections.unwrap_or(256) as usize;
        let encrypt_pool = crate::v4::crypto::pool::build_encrypt_pool(
            config.encrypt_workers.unwrap_or(0),
        );
        let pool_capacity = encrypt_pool.current_num_threads() * 2;

        Ok(Self {
            listener,
            socket_path: path.to_owned(),
            router_factory: Arc::new(router_factory),
            config,
            handshake_config,
            next_conn_id: AtomicU64::new(1),
            keypair,
            conn_semaphore: Arc::new(Semaphore::new(max_connections)),
            cancel_token: CancellationToken::new(),
            encrypt_pool,
            wire_pool: WireBufPool::new(pool_capacity),
            recv_pool: RecvBufPool::new(pool_capacity),
            counters,
            _marker: std::marker::PhantomData,
        })
    }

    pub async fn run(&self) -> Result<(), ServerError> {
        let local_creds = PeerCredentials::local();

        loop {
            let permit = Arc::clone(&self.conn_semaphore)
                .acquire_owned().await.expect("semaphore closed");

            tokio::select! {
                biased;
                _ = self.cancel_token.cancelled() => { drop(permit); return Ok(()); }
                accept_result = self.listener.accept() => {
                    let (mut stream, _) = match accept_result {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(error = %e, "accept failed");
                            drop(permit);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };

                    // UCred check
                    let peer_creds = match socket::extract_ucred(&stream) {
                        Ok(c) => c,
                        Err(e) => { tracing::error!(error = %e, "UCred failed"); continue; }
                    };
                    if peer_creds.uid != local_creds.uid {
                        tracing::error!(peer_uid = peer_creds.uid, "UID mismatch");
                        continue;
                    }

                    let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    let config = self.config.clone();
                    let child_token = self.cancel_token.child_token();
                    let router_factory = Arc::clone(&self.router_factory);

                    tracing::info!(conn_id, peer_pid = peer_creds.pid, "client connected");

                    // Prologue
                    let prologue = match crate::v4::crypto::noise::build_prologue(
                        local_creds.pid, local_creds.uid, peer_creds.pid, peer_creds.uid,
                    ) {
                        Ok(p) => p,
                        Err(e) => { tracing::error!(conn_id, error = ?e, "prologue failed"); continue; }
                    };

                    #[cfg(unix)]
                    socket::apply_socket_options(&stream, Some(262_144), Some(262_144));

                    // Handshake
                    let hr = match listener_handshake(
                        &mut stream, &self.keypair, self.handshake_config.clone(),
                        &prologue, Duration::from_secs(5),
                    ).await {
                        Ok(hr) => hr,
                        Err(e) => { tracing::error!(conn_id, error = ?e, "handshake failed"); continue; }
                    };

                    let keys = hr.keys.clone();
                    let session_id = hr.session_id;
                    let active_capabilities = hr.active_capabilities;
                    let agreed_clearance = hr.agreed_clearance;
                    let remote_peer_id = hr.remote_peer_id;
                    let encoder = Arc::new(hr.encoder);

                    // Arena bootstrap
                    #[cfg(target_os = "linux")]
                    let (arena_arc, sc_arc) = arena::create_arena(
                        &stream, &config, active_capabilities, conn_id,
                    );
                    #[cfg(target_os = "linux")]
                    let arena_bootstrap = arena::build_bootstrap(&arena_arc, &sc_arc, &config);
                    #[cfg(not(target_os = "linux"))]
                    let arena_bootstrap = ArenaBootstrap::None;

                    // Convert stream
                    let std_stream = stream.into_std().expect("into_std");
                    let raw_fd = { use std::os::unix::io::AsRawFd; std_stream.as_raw_fd() };
                    let owned_fd = Arc::new(std_stream);

                    // Phase 1: prepare connection
                    let mut prepared = connection::prepare_connection(
                        ConnectionParams {
                            raw_fd,
                            owned_fd: Arc::clone(&owned_fd),
                            encoder: Arc::clone(&encoder),
                            decoder: hr.decoder,
                            keys,
                            handshake_hash: hr.handshake_hash,
                            role: SessionRole::Listener,
                            agreed_aead: hr.agreed_aead,
                            session_id,
                            conn_id,
                            local_peer_id: hr.local_peer_id,
                            remote_peer_id,
                            agreed_clearance,
                            active_capabilities,
                            config: config.clone(),
                            encrypt_pool: Arc::clone(&self.encrypt_pool),
                            counters: Arc::clone(&self.counters),
                            wire_pool: self.wire_pool.clone(),
                            recv_pool: self.recv_pool.clone(),
                            thread_name_prefix: "rti-v4".into(),
                        },
                        arena_bootstrap,
                    );

                    // Construct StreamingSender + ConnectionHandle + router
                    // using outbound_tx from prepared (breaks circular dep)
                    #[cfg(target_os = "linux")]
                    let streaming_sender = arena_arc.as_ref().map(|a| {
                        crate::v4::streaming::send::StreamingSender::new(
                            Some(Arc::clone(a)), 0, sc_arc.clone(),
                            prepared.outbound_tx.clone(),
                        )
                    });

                    let conn_handle = ConnectionHandle {
                        outbound_tx: prepared.outbound_tx.clone(),
                        bulk_sender: prepared.bulk_sender.clone(),
                        #[cfg(target_os = "linux")]
                        streaming_sender,
                        conn_id, session_id,
                        peer_id: remote_peer_id,
                        agreed_clearance, active_capabilities,
                    };
                    let router: Arc<R> = Arc::new(router_factory(conn_handle));

                    let bulk_data_rx = prepared.bulk_data_rx.take()
                        .expect("bulk_data_rx already taken");
                    let bridge_router = Arc::clone(&router);
                    let bridge_info = prepared.connection_info.clone();
                    tokio::spawn(async move {
                        let mut bulk_data_rx = bulk_data_rx;
                        while let Some(signal) = bulk_data_rx.recv().await {
                            match signal {
                                crate::v4::io::control_loop::BulkDataSignal::Chunk { stream_id, chunk_index, data } => {
                                    crate::v4::bulk::counters::DIAG_RECV_DELIVERED_BYTES
                                        .fetch_add(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
                                    crate::v4::bulk::counters::DIAG_RECV_DELIVERED_FRAMES
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    bridge_router.on_bulk_chunk(&bridge_info, stream_id, chunk_index, &data);
                                }
                                crate::v4::io::control_loop::BulkDataSignal::Complete { .. } => {}
                            }
                        }
                    });

                    // Phase 2: spawn lane tasks with the real router
                    let runtime = connection::spawn_lanes(prepared, router);

                    // Connection lifetime management
                    tokio::spawn(async move {
                        tokio::select! {
                            biased;
                            _ = child_token.cancelled() => {}
                            _ = runtime.lane_cancel.cancelled() => {}
                        }
                        tracing::info!(conn_id, "session ended");
                        connection::shutdown_connection(runtime);
                        drop(owned_fd);
                        drop(permit);
                    });
                }
            }
        }
    }

    pub fn socket_path(&self) -> &Path { &self.socket_path }
    pub fn cancel_token(&self) -> &CancellationToken { &self.cancel_token }
    pub fn connection_count(&self) -> usize {
        let max = self.config.max_connections.unwrap_or(256) as usize;
        max - self.conn_semaphore.available_permits()
    }
    pub fn counters(&self) -> &Arc<crate::v4::bulk::counters::BulkCounters> { &self.counters }
}

impl<F, R> Drop for IpcServer<F, R>
where
    F: Fn(ConnectionHandle) -> R + Send + Sync + 'static,
    R: FrameRouter,
{
    fn drop(&mut self) {
        self.cancel_token.cancel();
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(path = %self.socket_path.display(), "IPC socket removed");
    }
}
