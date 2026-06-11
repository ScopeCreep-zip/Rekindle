//! IPC bus server — accepts connections, authenticates via UCred + Noise IK,
//! spawns per-connection task triples (read/control/write), and delivers
//! frames to the application through the FrameRouter trait.
//!
//! Each connection gets its own FrameRouter instance via a factory function.
//! The factory receives a `ConnectionHandle` containing the per-connection
//! outbound_tx and BulkSender so the router can send responses, events,
//! and bulk data back to the client. This is the symmetric peer architecture:
//! after handshake, both sides can send and receive all frame types.
//!
//! Per-connection task architecture:
//! 1. READ TASK: simple loop { read_exact }, no select!, sends to control loop
//! 2. CONTROL LOOP: biased select! with heartbeat/pong/deadline timers + frames
//! 3. WRITE TASK: biased select! draining lane channels onto the socket

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::v3::bulk::counters;
use crate::v3::context::ServerConfig;
use std::time::Duration;

use tokio::net::UnixListener;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::v3::bulk::send::BulkSender;
use crate::v3::context::{OutboundFrame, SessionConfig, SessionContext, SessionRole};
use crate::v3::io::control_loop;
use crate::v3::io::lane_channels::{LaneChannels, RecvBufPool, WireBufPool};
use crate::v3::router::FrameRouter;
use crate::v3::session::handshake::{listener_handshake, HandshakeConfig};
use crate::v3::socket::{self, PeerCredentials};
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;

/// Per-connection handle given to the router factory after handshake.
///
/// The factory uses this to construct a per-connection FrameRouter that
/// owns the outbound channels. When on_request fires, the router sends
/// DATAGRAM_REPLY through outbound_tx. When a large response is needed,
/// the router uses bulk_sender for rayon-parallel encryption.
#[derive(Clone)]
pub struct ConnectionHandle {
    /// Send control-plane frames (DATAGRAM_REPLY, events, lifecycle) to this client.
    pub outbound_tx: mpsc::Sender<OutboundFrame>,
    /// Send bulk data to this client via rayon parallel encryption.
    pub bulk_sender: BulkSender,
    /// Connection metadata.
    pub conn_id: u64,
    pub session_id: uuid::Uuid,
    pub peer_id: [u8; 32],
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
}

/// The IPC bus server.
///
/// Generic over `F` (router factory) and `R` (per-connection router).
/// The factory is called once per accepted connection after the handshake
/// succeeds. It receives a `ConnectionHandle` with per-connection channels
/// and returns a FrameRouter that owns those channels.
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
    /// Global wire buffer pool — shared across ALL connections.
    /// Capacity = rayon_workers × 2. Allocates on demand, not upfront.
    wire_pool: WireBufPool,
    /// Global recv buffer pool — shared across ALL connections.
    /// Capacity = rayon_workers × 2. Allocates on demand, not upfront.
    recv_pool: crate::v3::io::lane_channels::RecvBufPool,
    counters: Arc<crate::v3::bulk::counters::BulkCounters>,
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
    /// Bind to a Unix domain socket.
    ///
    /// `router_factory`: Called once per connection after handshake.
    /// Receives `ConnectionHandle` with per-connection outbound channels.
    /// Returns a FrameRouter that owns those channels.
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
        let encrypt_pool = crate::v3::crypto::pool::build_encrypt_pool(
            config.encrypt_workers.unwrap_or(0),
        );
        // Global buffer pools — shared across ALL connections.
        // Capacity = rayon_workers × 2: enough for full pipeline saturation
        // without over-provisioning. Buffers allocate on demand (no upfront
        // page faults), return to the pool on Drop.
        let pool_capacity = encrypt_pool.current_num_threads() * 2;
        let wire_pool = WireBufPool::new(pool_capacity);
        let recv_pool = RecvBufPool::new(pool_capacity);

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
            wire_pool,
            recv_pool,
            counters,
            _marker: std::marker::PhantomData,
        })
    }

    /// Run the accept loop. Spawns a task triple per connection. Runs until cancelled.
    pub async fn run(&self) -> Result<(), ServerError> {
        let local_creds = PeerCredentials::local();

        loop {
            let permit = Arc::clone(&self.conn_semaphore)
                .acquire_owned()
                .await
                .expect("semaphore closed");

            tokio::select! {
                biased;

                _ = self.cancel_token.cancelled() => {
                    drop(permit);
                    return Ok(());
                }

                accept_result = self.listener.accept() => {
                    let (mut stream, _addr) = match accept_result {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(error = %e, "accept failed");
                            drop(permit);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };

                    let peer_creds = match socket::extract_ucred(&stream) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = %e, "UCred extraction failed");
                            continue;
                        }
                    };
                    if peer_creds.uid != local_creds.uid {
                        tracing::error!(
                            peer_uid = peer_creds.uid,
                            local_uid = local_creds.uid,
                            "UID mismatch, dropping connection"
                        );
                        continue;
                    }

                    let conn_id = self.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    let config = self.config.clone();
                    let child_token = self.cancel_token.child_token();
                    let router_factory = Arc::clone(&self.router_factory);

                    tracing::info!(conn_id, peer_pid = peer_creds.pid, "client connected");

                    let prologue = match crate::v3::crypto::noise::build_prologue(
                        local_creds.pid, local_creds.uid,
                        peer_creds.pid, peer_creds.uid,
                    ) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!(conn_id, error = ?e, "prologue failed");
                            continue;
                        }
                    };

                    #[cfg(unix)]
                    socket::apply_socket_options(&stream, Some(262_144), Some(262_144));

                    let handshake_result = match listener_handshake(
                        &mut stream,
                        &self.keypair,
                        self.handshake_config.clone(),
                        &prologue,
                        Duration::from_secs(5),
                    ).await {
                        Ok(hr) => hr,
                        Err(e) => {
                            tracing::error!(conn_id, error = ?e, "handshake failed");
                            continue;
                        }
                    };

                    let keys = handshake_result.keys.clone();
                    let session_id = handshake_result.session_id;
                    let handshake_hash = handshake_result.handshake_hash;
                    let active_capabilities = handshake_result.active_capabilities;
                    let agreed_clearance = handshake_result.agreed_clearance;
                    let local_peer_id = handshake_result.local_peer_id;
                    let remote_peer_id = handshake_result.remote_peer_id;

                    let encoder = Arc::new(handshake_result.encoder);
                    let decoder = handshake_result.decoder;

                    // ── Sidechannel bootstrap (memfd handoff) ────────────
                    // Create SEQPACKET socketpair. Send one end to the client
                    // via SCM_RIGHTS on the main SOCK_STREAM. Both sides get
                    // one end of the sidechannel for memfd fd passing.
                    #[cfg(target_os = "linux")]
                    let handoff_coordinator = if active_capabilities.contains(CapabilityBits::HANDOFF_MEMFD) {
                        use std::os::unix::io::AsRawFd;
                        match crate::v3::handoff::sidechannel::SideChannel::create_pair() {
                            Ok((server_end, client_end)) => {
                                match crate::v3::handoff::sidechannel::send_fd_over_stream(
                                    stream.as_raw_fd(), client_end.fd()
                                ) {
                                    Ok(()) => {
                                        tracing::debug!(conn_id, "sidechannel: sent client fd via SCM_RIGHTS");
                                        drop(client_end);
                                        let (transport, memfd_ops) = crate::v3::handoff::transport::linux_pair(server_end);
                                        let credit = crate::v3::handoff::credit::SideChannelCreditTracker::new(
                                            config.initial_sidechannel_credit,
                                        );
                                        Some(crate::v3::handoff::coordinator::HandoffCoordinator::new(
                                            keys.handoff, credit, transport, memfd_ops,
                                        ))
                                    }
                                    Err(e) => {
                                        tracing::warn!(conn_id, error = ?e, "sidechannel: send failed — handoff disabled");
                                        None
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(conn_id, error = ?e, "sidechannel: socketpair failed — handoff disabled");
                                None
                            }
                        }
                    } else {
                        None
                    };

                    // Convert tokio UnixStream to std, wrap fd in OwnedFd for
                    // lifetime management. into_std() deregisters from epoll.
                    // The fd stays nonblocking — io_uring ops complete
                    // asynchronously regardless of blocking mode.
                    let std_stream = stream.into_std().expect("into_std failed");
                    let raw_fd = {
                        use std::os::unix::io::AsRawFd;
                        std_stream.as_raw_fd()
                    };
                    // OwnedFd ensures exactly one close via Drop. Stored in Arc
                    // so both the connection task (which drops it on shutdown to
                    // unblock uring tasks) and the uring tasks (which use the
                    // raw_fd copy) share the lifetime. The uring tasks use the
                    // Copy'd raw_fd i32 — they don't hold the Arc.
                    let owned_fd = Arc::new(std_stream);

                    // All lane channels are crossbeam bounded. The uring write task
                    // consumes crossbeam receivers directly — zero bridge threads.
                    let channel_capacity = 256;
                    let (lane_channels, bulk_wire_sender, lane_receivers) = LaneChannels::new(channel_capacity);
                    // ReadSignal: uring read task sends via crossbeam, bridged to
                    // tokio mpsc for the control loop's select!.
                    let (signal_tx, signal_rx) = mpsc::channel::<crate::v3::io::control_loop::ReadSignal>(256);
                    // Write errors: uring write task has no error channel — errors
                    // are fatal and the task exits. The control loop detects write
                    // task death via lane channel disconnection (ChannelClosed).
                    let (write_error_tx, write_error_rx) = mpsc::channel::<crate::v3::io::control_loop::WriteError>(4);
                    let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundFrame>(256);
                    let (sequenced_tx, sequenced_rx) = mpsc::channel::<crate::v3::context::SequencedOutbound>(64);
                    let (pending_fin_tx, pending_fin_rx) = mpsc::channel::<crate::v3::context::PendingFin>(64);
                    // Epoch signal for key rotation — shared between control loop
                    // (writer via SessionContext) and read task (reader).
                    // Created by SessionContext::from_fields, cloned here for the read task.
                    // AtomicPtr swap — no channel, no lock, no park.
                    // Audit queues — lock-free DispatchQueue + TokioWake.
                    // Rayon workers push LinkInputs; control loop pops via notified().
                    // Capacity = pool_capacity (rayon_workers × 2) so try_push is
                    // structurally infallible under normal operation.
                    // Audit queue capacity = channel_capacity. Every frame that goes
                    // through the bulk channel produces exactly one audit link. The queue
                    // must hold at least as many links as the bulk channel holds frames.
                    let (outbound_audit_queue, outbound_audit_wake) = crate::v3::bulk::new_audit_queue(channel_capacity);
                    let (inbound_audit_queue, inbound_audit_wake) = crate::v3::bulk::new_audit_queue(channel_capacity);

                    let encrypt_pool = Arc::clone(&self.encrypt_pool);
                    let counters = Arc::clone(&self.counters);

                    // Clone the global wire buffer pool — all connections share
                    // one pool sized to rayon_workers × 2. No per-connection allocation.
                    let wire_pool = self.wire_pool.clone();

                    // Extract the raw crossbeam sender. Clone for the control loop
                    // (OPEN coalescing), original goes to BulkSender.
                    let bulk_crossbeam_tx = bulk_wire_sender.into_crossbeam_sender();
                    let bulk_wire_tx_for_control = bulk_crossbeam_tx.clone();

                    // Per-connection BulkSender for server→client bulk transfers
                    let bulk_sender = BulkSender::new(
                        Arc::clone(&encoder),
                        Arc::clone(&encrypt_pool),
                        bulk_crossbeam_tx,
                        Arc::clone(&outbound_audit_queue),
                        sequenced_tx,
                        pending_fin_tx,
                        wire_pool,
                    );

                    // Per-connection BulkReceiver for parallel inbound decrypt
                    let inbound_cipher = crate::v3::session::handshake::build_bulk_cipher(
                        handshake_result.agreed_aead,
                        &keys.stream_d2l,
                        crate::v3::wire::constants::DIRECTION_ID_D2L,
                    ).expect("inbound cipher init");
                    let recv_result_wake = rekindle_transport_buff::adapters::tokio::TokioWake::new();
                    let recv_result_queue = std::sync::Arc::new(
                        rekindle_transport_buff::DispatchQueue::new(64, recv_result_wake.clone()),
                    );
                    // Clone the global recv buffer pool — all connections share
                    // one pool sized to rayon_workers × 2. No per-connection allocation.
                    let recv_buf_pool = self.recv_pool.clone();
                    let bulk_receiver = crate::v3::bulk::recv::BulkReceiver::new(
                        keys.envelope_d2l,
                        keys.header_d2l,
                        Arc::new(inbound_cipher),
                        Arc::clone(&encrypt_pool),
                        Arc::clone(&recv_result_queue),
                        Arc::clone(&inbound_audit_queue),
                        recv_buf_pool,
                        self.wire_pool.clone(),
                    );
                    // CreditGuard: recv admission control.
                    let credit_guard = Arc::new(
                        rekindle_transport_buff::CreditGuard::new(config.max_pending_bytes_per_session),
                    );

                    // Bulk data channel: control loop → drain task.
                    let (bulk_data_tx, mut bulk_data_rx) =
                        mpsc::channel::<control_loop::BulkDataSignal>(64);
                    tokio::spawn(async move {
                        while let Some(signal) = bulk_data_rx.recv().await {
                            if let control_loop::BulkDataSignal::Chunk { data, .. } = &signal {
                                counters::DIAG_RECV_DELIVERED_BYTES.fetch_add(data.len() as u64, Ordering::Relaxed);
                                counters::DIAG_RECV_DELIVERED_FRAMES.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    });

                    // Create per-connection router via factory.
                    // The router owns outbound_tx and bulk_sender for sending
                    // responses, events, and bulk data back to this client.
                    let conn_handle = ConnectionHandle {
                        outbound_tx,
                        bulk_sender,
                        conn_id,
                        session_id,
                        peer_id: remote_peer_id,
                        agreed_clearance,
                        active_capabilities,
                    };
                    let router: Arc<R> = Arc::new(router_factory(conn_handle));
                    let bulk_threshold = config.bulk_decrypt_threshold.unwrap_or(65536);

                    let agreed_aead = handshake_result.agreed_aead;
                    let mut ctx = SessionContext::from_fields(
                        session_id,
                        local_peer_id,
                        remote_peer_id,
                        agreed_clearance,
                        active_capabilities,
                        handshake_hash,
                        keys,
                        config,
                        router,
                        conn_id,
                        SessionRole::Listener,
                        agreed_aead,
                    );
                    #[cfg(target_os = "linux")]
                    if let Some(coord) = handoff_coordinator {
                        ctx.set_handoff_coordinator(coord);
                    }
                    let epoch_signal_for_read = Arc::clone(ctx.epoch_signal());
                    let encoder_for_read = Arc::clone(&encoder);

                    // Spawn per-connection task triple: uring write, uring read, control loop.
                    // io_uring tasks run on dedicated std::threads (SINGLE_ISSUER
                    // requires the ring to be created on the submitting thread).
                    // The control loop runs on tokio (needs timers, select!).
                    // Bridge handles are captured by the async move block and held
                    // alive for the connection's lifetime.
                    tracing::debug!(conn_id, "spawning per-connection task triple");
                    tokio::spawn(async move {
                        tracing::debug!(conn_id, "per-connection task triple: entered spawn");
                        let last_activity_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));

                        // Shutdown eventfd — the read task registers a PollAdd SQE
                        // on this fd. The shutdown path writes to it to wake the
                        // read task's submit_and_wait when DEFER_TASKRUN is active.
                        let shutdown_eventfd = crate::v3::io::uring_read_task::create_shutdown_eventfd();

                        // io_uring write task — std::thread, consumes crossbeam
                        // lane receivers directly. Holds Arc clone of owned_fd
                        // to keep the socket alive until the thread exits.
                        let write_counters = Arc::clone(&counters);
                        let write_fd_hold = Arc::clone(&owned_fd);
                        let write_handle = std::thread::Builder::new()
                            .name(format!("rti-v3-write-{conn_id}"))
                            .spawn(move || {
                                crate::v3::io::uring_write_task::run(
                                    raw_fd,
                                    lane_receivers.control_rx,
                                    lane_receivers.audit_rx,
                                    lane_receivers.handoff_rx,
                                    lane_receivers.data_rx,
                                    lane_receivers.bulk_rx,
                                    write_counters,
                                    write_error_tx,
                                );
                                drop(write_fd_hold);
                            })
                            .expect("failed to spawn uring write task");

                        // io_uring read task — std::thread, sends ReadSignal
                        // through crossbeam → bridge → tokio mpsc for control loop.
                        // Holds Arc clone of owned_fd for fd lifetime.
                        let read_counters = Arc::clone(&counters);
                        let read_fd_hold = Arc::clone(&owned_fd);
                        let read_credit_guard = Arc::clone(&credit_guard);
                        let read_handle = std::thread::Builder::new()
                            .name(format!("rti-v3-read-{conn_id}"))
                            .spawn(move || {
                                crate::v3::io::uring_read_task::run(
                                    raw_fd,
                                    decoder,
                                    signal_tx,
                                    bulk_receiver,
                                    bulk_threshold,
                                    read_counters,
                                    read_credit_guard,
                                    shutdown_eventfd,
                                    epoch_signal_for_read,
                                    encoder_for_read,
                                );
                                drop(read_fd_hold);
                            })
                            .expect("failed to spawn uring read task");

                        let outcome = tokio::select! {
                            biased;
                            _ = child_token.cancelled() => {
                                crate::v3::io::read_task::SessionOutcome::Closed { peer_initiated: false }
                            }
                            result = control_loop::run(
                                ctx, signal_rx, lane_channels, encoder, encrypt_pool,
                                Arc::clone(&counters),
                                write_error_rx, outbound_rx,
                                outbound_audit_queue, outbound_audit_wake,
                                inbound_audit_queue, inbound_audit_wake,
                                recv_result_queue, sequenced_rx, pending_fin_rx,
                                bulk_data_tx,
                                credit_guard,
                                last_activity_ns,
                                bulk_wire_tx_for_control,
                            ) => {
                                result
                            }
                        };

                        tracing::info!(conn_id, outcome = ?outcome, "session ended — beginning shutdown sequence");

                        // Signal the shutdown eventfd BEFORE closing the socket fd.
                        // The read task's PollAdd SQE generates a CQE with TAG_SHUTDOWN,
                        // waking submit_and_wait even with DEFER_TASKRUN active.
                        tracing::debug!(conn_id, "shutdown: signaling read task eventfd");
                        crate::v3::io::uring_read_task::signal_shutdown(shutdown_eventfd);

                        // Do NOT drop owned_fd here. The uring threads hold a
                        // Copy'd RawFd. The read task exits via eventfd signal.
                        // The write task exits via crossbeam disconnection. The
                        // fd closes naturally when owned_fd drops at the end of
                        // this block — after all threads have been joined.

                        tracing::debug!(conn_id, "shutdown: joining read thread");
                        let _ = read_handle.join();
                        tracing::debug!(conn_id, "shutdown: read thread joined");

                        // Close the shutdown eventfd — no longer needed after
                        // read thread has joined.
                        crate::v3::io::uring_read_task::close_shutdown_eventfd(shutdown_eventfd);

                        tracing::debug!(conn_id, "shutdown: joining write thread");
                        let _ = write_handle.join();
                        tracing::debug!(conn_id, "shutdown: write thread joined");

                        // No bridge threads to join — OS threads send directly to tokio mpsc.

                        // Drop owned_fd AFTER all threads joined — the uring
                        // threads held a Copy'd RawFd that is only valid while
                        // owned_fd keeps the underlying UnixStream alive.
                        drop(owned_fd);

                        tracing::info!(conn_id, outcome = ?outcome, "shutdown: complete");
                        drop(permit);
                    });
                }
            }
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    pub fn connection_count(&self) -> usize {
        let max = self.config.max_connections.unwrap_or(256) as usize;
        max - self.conn_semaphore.available_permits()
    }

    /// Observability counters — shared across all connections.
    /// Read by status endpoints, Prometheus exporters, TUI dashboards.
    pub fn counters(&self) -> &Arc<crate::v3::bulk::counters::BulkCounters> {
        &self.counters
    }
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
