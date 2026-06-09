//! IPC bus client — connects to a server, performs Noise IK handshake,
//! spawns task triple (read/control/write), exposes typed send/recv API.
//!
//! The handshake returns ready-to-use `FrameEncoder` and `FrameDecoder`
//! with correct direction, algorithm, and keys. The client never selects
//! keys, directions, or algorithms — the handshake is the SSOT.
//!
//! Send operations (`send_request`, `send_bulk`, etc.) take `&self` —
//! multiple concurrent sends are safe. Recv operations (`recv`, `recv_bulk`)
//! take `&self` — concurrent send+recv is safe via internal Mutex.

pub mod types;
pub mod send;
pub mod recv;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::v3::context::{SessionConfig, SessionContext, SessionRole};
use crate::v3::io::lane_channels::RecvBufPool;
use crate::v3::io::control_loop;
use crate::v3::io::lane_channels::{LaneChannels, WireBufPool};
use crate::v3::router::{ConnectionInfo, FrameRouter};
use crate::v3::session::handshake::{
    dialler_handshake, HandshakeConfig, HandshakeError,
};
use crate::v3::socket::{self, PeerCredentials};
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;
use crate::v3::wire::frame_class::FrameClass;
use crate::v3::wire::frame_kind::DatagramKind;

use send::SendState;
use recv::InboundReceiver;

impl From<HandshakeError> for ClientError {
    fn from(e: HandshakeError) -> Self {
        Self::Handshake(format!("{e:?}"))
    }
}

// ── ClientRouter ─────────────────────────────────────────────────
//
// Production FrameRouter for the client side. Forwards inbound frames
// to channels consumed by the client's recv API, and resolves pending
// ack/bulk waiters for the client's send API.

struct ClientRouter {
    inbound_tx: mpsc::Sender<InboundFrame>,
    pending_acks: Arc<Mutex<HashMap<uuid::Uuid, send::AckWaiter>>>,
    pending_bulk: Arc<Mutex<HashMap<u8, send::BulkWaiter>>>,
}

impl FrameRouter for ClientRouter {
    fn route_frame(&self, info: &ConnectionInfo, class: u8, kind: u8, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class, kind,
            payload: payload.to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: None,
            sender_clearance: None,
            subscription_id: None,
            topic_hash: None,
            event_seq: None,
            status_phase: None,
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_request(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Request as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: Some(message_id),
            sender_clearance: Some(sender_clearance),
            subscription_id: None,
            topic_hash: None,
            event_seq: None,
            status_phase: None,
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_notify(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Notify as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: Some(message_id),
            sender_clearance: Some(sender_clearance),
            subscription_id: None,
            topic_hash: None,
            event_seq: None,
            status_phase: None,
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_publish(&self, info: &ConnectionInfo, subscription_id: uuid::Uuid, topic_hash: &[u8; 32], event_seq: u32, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Publish as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: None,
            sender_clearance: None,
            subscription_id: Some(subscription_id),
            topic_hash: Some(*topic_hash),
            event_seq: Some(event_seq),
            status_phase: None,
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_reply(&self, info: &ConnectionInfo, reply_message_id: uuid::Uuid, correlation_id: uuid::Uuid, status_phase: u32, payload: &[u8]) {
        if let Some(waiter) = self.pending_acks.lock().remove(&correlation_id) {
            let _ = waiter.tx.send(Ok(()));
        }
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Reply as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: Some(reply_message_id),
            sender_clearance: None,
            subscription_id: None,
            topic_hash: None,
            event_seq: None,
            status_phase: Some(status_phase),
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_reject(&self, info: &ConnectionInfo, rejected_message_id: uuid::Uuid, reason_code: u32, detail: &str) {
        if let Some(waiter) = self.pending_acks.lock().remove(&rejected_message_id) {
            let _ = waiter.tx.send(Err(SendError::Rejected {
                reason_code,
                detail: detail.to_owned(),
            }));
        }
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v3::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Reject as u8,
            payload: detail.as_bytes().to_vec(),
            conn_id: info.conn_id,
            session_id: info.session_id,
            peer_id: info.peer_id,
            message_id: Some(rejected_message_id),
            sender_clearance: None,
            subscription_id: None,
            topic_hash: None,
            event_seq: None,
            status_phase: None,
            transfer_id: None,
            stream_id: None,
            chunk_index: None,
        });
    }

    fn on_bulk_complete(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32) {
        tracing::info!(
            conn_id = info.conn_id,
            stream_id, transfer_id = %transfer_id,
            total_bytes, total_chunks,
            "ClientRouter::on_bulk_complete — resolving pending_bulk"
        );
        // Completion signal goes through bulk_data_tx → bridge task → bulk_chunk_tx
        // to maintain ordering with data chunks. Do NOT send to bulk_chunk_tx directly.
        if let Some(waiter) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = waiter.tx.send(Ok(BulkDelivered {
                bytes_transferred: total_bytes,
                duration: waiter.started_at.elapsed(),
                chunks: total_chunks as u64,
            }));
        }
    }

    fn on_bulk_failed(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, reason: &str) {
        tracing::warn!(
            conn_id = info.conn_id,
            session_id = %info.session_id,
            stream_id,
            transfer_id = %transfer_id,
            reason,
            "bulk transfer failed"
        );
        if let Some(waiter) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = waiter.tx.send(Err(BulkError::PeerRejected {
                reason: reason.to_owned(),
            }));
        }
    }

    fn on_ack(&self, info: &ConnectionInfo, message_ids: &[uuid::Uuid]) {
        tracing::trace!(
            conn_id = info.conn_id,
            count = message_ids.len(),
            "ack received"
        );
        let mut pending = self.pending_acks.lock();
        for message_id in message_ids {
            if let Some(waiter) = pending.remove(message_id) {
                let _ = waiter.tx.send(Ok(()));
            }
        }
    }

    fn on_connection_state_change(&self, info: &ConnectionInfo, old_state: &str, new_state: &str) {
        tracing::info!(
            conn_id = info.conn_id,
            session_id = %info.session_id,
            old_state, new_state,
            "client connection state change"
        );
    }
}

// ── IpcClient ────────────────────────────────────────────────────

/// Cancels the control loop and signals the uring read task on drop.
/// Extracted from IpcClient so that IpcClient itself has no Drop impl,
/// enabling partial moves in shutdown().
struct ClientGuard {
    cancel: tokio_util::sync::CancellationToken,
    /// Shutdown eventfd for the uring read task. Signaled on drop to
    /// wake submit_and_wait when DEFER_TASKRUN is active. -1 means
    /// already consumed by shutdown() (which signals and closes it).
    shutdown_eventfd: std::sync::atomic::AtomicI32,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        // Signal the read task's shutdown eventfd BEFORE cancel.
        // On the crash path (Drop without shutdown), this wakes the
        // uring read task's submit_and_wait so it can exit cleanly.
        // On the graceful path, shutdown() already swapped this to -1.
        let efd = self.shutdown_eventfd.swap(-1, std::sync::atomic::Ordering::AcqRel);
        if efd >= 0 {
            crate::v3::io::uring_read_task::signal_shutdown(efd);
            // Close the eventfd after signaling. The read task's PollAdd
            // CQE has already been triggered by the signal write. Closing
            // the fd prevents accumulation at scale (200K connections).
            crate::v3::io::uring_read_task::close_shutdown_eventfd(efd);
        }

        // Cancel the control loop via the token. This cascades:
        // 1. control loop's select sees cancelled() → returns → lane_channels dropped
        //    → crossbeam senders disconnect → uring write task's recv returns Disconnected
        // 2. control loop returns → signal bridge's tokio mpsc sender drops
        //    → bridge thread exits → crossbeam signal_tx drops
        // 3. owned_fd drops → close(fd) → uring tasks see -EBADF on any in-flight SQEs
        self.cancel.cancel();
    }
}

/// The IPC bus client.
///
/// Send via `&self` (concurrent-safe). Recv via `&mut self` (exclusive).
/// Shutdown is consuming — `shutdown(self)` drops all resources.
///
/// No Drop impl on IpcClient itself — ClientGuard handles cancellation.
/// This enables partial moves in shutdown() so BulkSender's crossbeam
/// senders can be dropped before joining bridge threads.
pub struct IpcClient {
    // ── Drop order is declaration order. Correct sequence: ──────────
    // 1. send_state: drops BulkSender's tokio mpsc senders
    // 2. recv_state: drops inbound channels
    // 3. metadata fields (Copy/Clone, no side effects)
    // 4. control_handle: JoinHandle detaches (task sees cancel on next poll)
    // 5. guard: cancels token → control loop returns → LaneChannels drop
    //    → all crossbeam senders disconnect. Signals shutdown eventfd
    //    → uring read task exits.
    // 6. owned_fd: closes socket fd LAST — uring threads (exited by
    //    step 5) held Copy'd RawFd that was valid until this drop.
    send_state: SendState,
    recv_state: InboundReceiver,
    session_id: uuid::Uuid,
    agreed_clearance: Clearance,
    active_capabilities: CapabilityBits,
    phase: Arc<Mutex<ConnectionPhase>>,
    /// Some during normal operation. Taken by shutdown() to await task exit.
    control_handle: Option<tokio::task::JoinHandle<()>>,
    /// Guard signals eventfd + cancels token on drop.
    guard: ClientGuard,
    /// Socket fd ownership anchor. MUST be the LAST field — drops after
    /// guard (which signals uring threads to exit) and after bridges.
    /// The uring threads hold a Copy'd RawFd that is only valid while
    /// this Arc keeps the UnixStream alive.
    #[cfg(target_os = "linux")]
    owned_fd: Option<Arc<std::os::unix::net::UnixStream>>,
}

impl IpcClient {
    /// Connect to the IPC server, perform Noise IK handshake, spawn task triple.
    ///
    /// `path`: Unix socket path.
    /// `server_public_key`: The server's known static X25519 public key.
    /// `client_keypair`: The client's pre-existing static keypair for identity.
    /// `config`: Session tunables (heartbeat interval, credit limits, etc.).
    /// `handshake_config`: Capabilities and clearance for negotiation.
    pub async fn connect(
        path: &Path,
        server_public_key: &[u8; 32],
        client_keypair: &snow::Keypair,
        config: SessionConfig,
        handshake_config: HandshakeConfig,
    ) -> Result<Self, ClientError> {
        let mut stream = tokio::net::UnixStream::connect(path).await
            .map_err(ClientError::Connect)?;

        // UCred extraction for prologue binding
        let peer_creds = socket::extract_ucred(&stream)
            .map_err(|e| ClientError::UcredFailed(format!("{e}")))?;
        let local_creds = PeerCredentials::local();
        let prologue = crate::v3::crypto::noise::build_prologue(
            local_creds.pid, local_creds.uid,
            peer_creds.pid, peer_creds.uid,
        ).map_err(|e| ClientError::PrologueFailed(format!("{e:?}")))?;

        // Socket buffer tuning
        #[cfg(unix)]
        socket::apply_socket_options(&stream, Some(262_144), Some(262_144));

        // Noise IK handshake on unsplit stream.
        // Returns ready-to-use encoder (d2l) and decoder (l2d).
        let handshake_result = dialler_handshake(
            &mut stream,
            client_keypair,
            server_public_key,
            handshake_config,
            &prologue,
            Duration::from_secs(5),
        ).await?;

        // Extract metadata before moving encoder/decoder
        let session_id = handshake_result.session_id;
        let agreed_clearance = handshake_result.agreed_clearance;
        let active_capabilities = handshake_result.active_capabilities;
        let keys = handshake_result.keys.clone();
        let handshake_hash = handshake_result.handshake_hash;
        let local_peer_id = handshake_result.local_peer_id;
        let remote_peer_id = handshake_result.remote_peer_id;

        let encoder = Arc::new(handshake_result.encoder);
        let decoder = handshake_result.decoder;
        let encrypt_pool = crate::v3::crypto::pool::build_encrypt_pool(
            config.encrypt_workers.unwrap_or(0),
        );

        // ── Receive sidechannel fd from server (memfd handoff) ───────
        // The server created a SEQPACKET socketpair and sent one end via
        // SCM_RIGHTS. Receive it before converting the stream to io_uring.
        #[cfg(target_os = "linux")]
        let handoff_coordinator = if active_capabilities.contains(crate::v3::wire::capability::CapabilityBits::HANDOFF_MEMFD) {
            use std::os::unix::io::AsRawFd;
            match crate::v3::handoff::sidechannel::recv_fd_from_stream(stream.as_raw_fd()) {
                Ok(sc_fd) => {
                    tracing::debug!("sidechannel: received fd via SCM_RIGHTS");
                    let sidechannel = crate::v3::handoff::sidechannel::SideChannel::from_raw_fd(sc_fd);
                    let (transport, memfd_ops) = crate::v3::handoff::transport::linux_pair(sidechannel);
                    let credit = crate::v3::handoff::credit::SideChannelCreditTracker::new(
                        config.initial_sidechannel_credit,
                    );
                    Some(crate::v3::handoff::coordinator::HandoffCoordinator::new(
                        keys.handoff, credit, transport, memfd_ops,
                    ))
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "sidechannel: recv failed — handoff disabled");
                    None
                }
            }
        } else {
            None
        };

        // Convert tokio UnixStream to std, extract RawFd for io_uring tasks.
        let std_stream = stream.into_std().map_err(|e| ClientError::Connect(e))?;
        let raw_fd = {
            use std::os::unix::io::AsRawFd;
            std_stream.as_raw_fd()
        };
        let owned_fd = Arc::new(std_stream);

        // All lane channels are crossbeam bounded. Uring write task consumes directly.
        let channel_capacity = 256;
        let (lane_channels, bulk_wire_sender, lane_receivers) = LaneChannels::new(channel_capacity);
        // ReadSignal: uring read task → tokio mpsc → control loop (direct, no bridge)
        let (signal_tx, signal_rx) = mpsc::channel::<crate::v3::io::control_loop::ReadSignal>(256);
        let (write_error_tx, write_error_rx) = mpsc::channel::<crate::v3::io::control_loop::WriteError>(4);
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let (bulk_chunk_tx, bulk_chunk_rx) = mpsc::channel(64);
        // Staging channel: client API → control loop → lane_channels → write task.
        let (outbound_tx, outbound_rx) = mpsc::channel(256);
        // Sequenced channel: BulkSender STREAM_OPEN with confirmation
        let (sequenced_tx, sequenced_rx) = mpsc::channel::<crate::v3::context::SequencedOutbound>(64);
        let sequenced_tx_for_client = sequenced_tx.clone();
        // PendingFin channel: BulkSender → control loop (signals when to emit STREAM_FIN)
        let (pending_fin_tx, pending_fin_rx) = mpsc::channel::<crate::v3::context::PendingFin>(64);
        // Epoch signal for key rotation — created by SessionContext,
        // cloned for the read task. AtomicPtr swap, no channel.
        // Audit queues — lock-free DispatchQueue + TokioWake.
        // Rayon workers push LinkInputs; control loop pops via notified().
        // Capacity = channel_capacity: every bulk frame produces one audit link.
        let (outbound_audit_queue, outbound_audit_wake) = crate::v3::bulk::new_audit_queue(channel_capacity);
        let (inbound_audit_queue, inbound_audit_wake) = crate::v3::bulk::new_audit_queue(channel_capacity);

        let phase = Arc::new(Mutex::new(ConnectionPhase::Ready));
        let phase_control = Arc::clone(&phase);
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let child_cancel = cancel_token.child_token();

        let pending_acks = Arc::new(Mutex::new(HashMap::new()));
        let pending_bulk = Arc::new(Mutex::new(HashMap::new()));
        let pending_acks_ctrl = Arc::clone(&pending_acks);
        let pending_bulk_ctrl = Arc::clone(&pending_bulk);

        // BulkSender: dispatches STREAM_PAYLOAD chunks directly to rayon → write task,
        // bypassing the control loop. STREAM_OPEN/FIN go through outbound_tx for lifecycle.
        // Pool capacity = rayon_workers × 2: enough for full pipeline saturation.
        let pool_capacity = encrypt_pool.current_num_threads() * 2;
        let wire_pool = WireBufPool::new(pool_capacity);
        let bulk_crossbeam_tx = bulk_wire_sender.into_crossbeam_sender();
        let bulk_wire_tx_for_control = bulk_crossbeam_tx.clone();

        let bulk_sender = crate::v3::bulk::send::BulkSender::new(
            Arc::clone(&encoder),
            Arc::clone(&encrypt_pool),
            bulk_crossbeam_tx,
            Arc::clone(&outbound_audit_queue),
            sequenced_tx,
            pending_fin_tx,
            wire_pool.clone(),
        );

        // BulkReceiver for parallel inbound decrypt (server→client bulk)
        let inbound_cipher = crate::v3::session::handshake::build_bulk_cipher(
            handshake_result.agreed_aead,
            &keys.stream_l2d,
            crate::v3::wire::constants::DIRECTION_ID_L2D,
        ).map_err(|e| ClientError::CipherInit(format!("{e:?}")))?;
        let recv_result_wake = rekindle_transport_buff::adapters::tokio::TokioWake::new();
        let recv_result_queue = std::sync::Arc::new(
            rekindle_transport_buff::DispatchQueue::new(64, recv_result_wake.clone()),
        );
        let recv_buf_pool = RecvBufPool::new(pool_capacity);
        let bulk_receiver = crate::v3::bulk::recv::BulkReceiver::new(
            keys.envelope_l2d,
            keys.header_l2d,
            Arc::new(inbound_cipher),
            Arc::clone(&encrypt_pool),
            Arc::clone(&recv_result_queue),
            Arc::clone(&inbound_audit_queue),
            recv_buf_pool,
            wire_pool.clone(),
        );
        let bulk_threshold = config.bulk_decrypt_threshold.unwrap_or(65536);

        // CreditGuard: recv admission control.
        let credit_guard = Arc::new(
            rekindle_transport_buff::CreditGuard::new(config.max_pending_bytes_per_session),
        );

        // Bulk data channel: control loop → bridge task → bulk_chunk_tx
        let (bulk_data_tx, mut bulk_data_rx) =
            mpsc::channel::<control_loop::BulkDataSignal>(64);
        tokio::spawn(async move {
            while let Some(signal) = bulk_data_rx.recv().await {
                let chunk = match signal {
                    control_loop::BulkDataSignal::Chunk { stream_id, chunk_index, data } => {
                        BulkChunk::from_plaintext(stream_id, chunk_index, data)
                    }
                    control_loop::BulkDataSignal::Complete { stream_id, total_chunks, .. } => {
                        BulkChunk::completion(stream_id, total_chunks)
                    }
                };
                if bulk_chunk_tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        let handoff_enabled = Arc::new(std::sync::atomic::AtomicBool::new(
            active_capabilities.contains(crate::v3::wire::capability::CapabilityBits::HANDOFF_MEMFD),
        ));

        let send_state = SendState {
            outbound_tx,
            sequenced_tx: sequenced_tx_for_client,
            bulk_sender,
            pending_acks,
            pending_bulk,
            phase: Arc::clone(&phase),
            agreed_clearance,
            handoff_threshold: config.handoff_threshold_bytes,
            handoff_enabled,
        };

        // Build recv state
        let cancelled_recv_streams = Arc::new(Mutex::new(HashSet::new()));
        let recv_state = InboundReceiver::new(
            inbound_rx, bulk_chunk_rx, Arc::clone(&cancelled_recv_streams),
        );

        // Client-side router: forwards inbound frames to channels for the
        // client's recv API, and resolves pending ack/bulk waiters.
        let client_router = Arc::new(ClientRouter {
            inbound_tx,
            pending_acks: pending_acks_ctrl,
            pending_bulk: pending_bulk_ctrl,
        });

        // SessionContext for the client's control loop
        let agreed_aead = handshake_result.agreed_aead;
        let mut ctx = SessionContext::from_fields(
            session_id, local_peer_id, remote_peer_id,
            agreed_clearance, active_capabilities, handshake_hash,
            keys, config, client_router, 0,
            SessionRole::Dialler,
            agreed_aead,
        );
        #[cfg(target_os = "linux")]
        if let Some(coord) = handoff_coordinator {
            ctx.set_handoff_coordinator(coord);
        }
        let epoch_signal_for_read = std::sync::Arc::clone(ctx.epoch_signal());
        let encoder_for_read = Arc::clone(&encoder);
        let counters = crate::v3::bulk::counters::BulkCounters::new();

        // Shared activity tracker — read task writes on every socket frame,
        // control loop heartbeat reads. Decoupled from control loop load.
        let last_activity_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Shutdown eventfd for the read task's DEFER_TASKRUN wakeup.
        let shutdown_eventfd = crate::v3::io::uring_read_task::create_shutdown_eventfd();

        // io_uring write task — std::thread, consumes crossbeam lane receivers.
        // Holds an Arc clone of owned_fd to keep the socket alive until
        // the thread exits. The thread uses raw_fd for io_uring ops.
        let write_counters = Arc::clone(&counters);
        let write_fd_hold = Arc::clone(&owned_fd);
        std::thread::Builder::new()
            .name("rti-v3-client-write".into())
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

        // io_uring read task — std::thread, sends ReadSignal via crossbeam.
        // Holds an Arc clone of owned_fd to keep the socket alive until
        // the thread exits.
        let read_counters = Arc::clone(&counters);
        let read_fd_hold = Arc::clone(&owned_fd);
        let read_credit_guard = Arc::clone(&credit_guard);
        std::thread::Builder::new()
            .name("rti-v3-client-read".into())
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

        // Control loop on tokio — needs timers, select!, async channels.
        let control_handle = tokio::spawn(async move {
            let outcome = tokio::select! {
                biased;
                _ = child_cancel.cancelled() => {
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
                    Arc::clone(&credit_guard),
                    last_activity_ns,
                    bulk_wire_tx_for_control,
                ) => {
                    result
                }
            };
            *phase_control.lock() = ConnectionPhase::Closed;
            tracing::info!(outcome = ?outcome, "client session ended");
        });

        Ok(Self {
            send_state,
            recv_state,
            session_id,
            agreed_clearance,
            active_capabilities,
            phase,
            control_handle: Some(control_handle),
            guard: ClientGuard {
                cancel: cancel_token,
                shutdown_eventfd: std::sync::atomic::AtomicI32::new(shutdown_eventfd),
            },
            #[cfg(target_os = "linux")]
            owned_fd: Some(owned_fd),
        })
    }

    // ── Metadata accessors ────────────────────────────────────────

    pub fn session_id(&self) -> uuid::Uuid { self.session_id }
    pub fn agreed_clearance(&self) -> Clearance { self.agreed_clearance }
    pub fn active_capabilities(&self) -> CapabilityBits { self.active_capabilities }
    pub fn phase(&self) -> ConnectionPhase { *self.phase.lock() }

    // ── Send operations (delegate to SendState, take &self) ──────

    pub async fn send_request(&self, payload: &[u8], ack_timeout: Duration) -> Result<SendDelivered, SendError> {
        self.send_state.send_request(payload, ack_timeout).await
    }

    pub async fn send_notify(&self, payload: &[u8]) -> Result<(), ClientError> {
        self.send_state.send_notify(payload).await
    }

    pub async fn send_bulk(&self, stream_id: u8, payload: &[u8], ack_timeout: Duration) -> Result<BulkDelivered, BulkError> {
        self.send_state.send_bulk(stream_id, payload, ack_timeout).await
    }

    /// Initiate key rotation. Both sides derive new HKDF keys from fresh
    /// ephemeral secrets. Returns Ok after ROTATE_COMMIT is processed and
    /// both sides have matching new keys. All subsequent frames use new keys.
    pub async fn rotate_keys(&self, timeout: Duration) -> Result<(), BulkError> {
        self.send_state.rotate_keys(timeout).await
            .map_err(|e| BulkError::PeerRejected { reason: format!("{e:?}") })
    }

    pub async fn cancel_bulk(&self, stream_id: u8) {
        self.send_state.cancel_bulk(stream_id).await;
    }

    // ── Recv operations (delegate to InboundReceiver, take &mut self) ──

    pub async fn recv(&self) -> Option<InboundFrame> {
        self.recv_state.recv().await
    }

    pub async fn recv_bulk_chunk(&self) -> Option<BulkChunk> {
        self.recv_state.recv_bulk_chunk().await
    }

    pub async fn recv_bulk(&self) -> Option<(u8, Vec<u8>)> {
        self.recv_state.recv_bulk().await
    }

    pub fn cancel_recv_bulk(&self, stream_id: u8) {
        self.recv_state.cancel_recv_bulk(stream_id);
    }

    // ── Raw outbound (symmetric peer API) ────────────────────────

    /// Send a raw OutboundFrame through the control loop.
    /// Used by the application layer to send DATAGRAM_REPLY, events,
    /// and any frame type back through the same connection.
    /// This is the symmetric peer API — both client and server can
    /// send any frame type after handshake.
    pub async fn send_raw_outbound(&self, frame: crate::v3::context::OutboundFrame) -> Result<(), ClientError> {
        self.send_state.outbound_tx.send(frame).await
            .map_err(|e| ClientError::Send(format!("{e:?}")))
    }

    // ── Lifecycle ─────────────────────────────────────────────────

    /// Graceful shutdown: send GOODBYE, await session close, join bridge threads.
    ///
    /// 1. Sends GOODBYE through the control loop (transitions to Draining)
    /// 2. Awaits the control loop task with 5s timeout (server sends
    ///    GOODBYE_ACK → BothGoodbyeAcked → Closed → task exits)
    /// 3. If timeout: cancels via token, forcing immediate exit
    /// 4. Drops send_state (BulkSender's crossbeam senders disconnect)
    /// 5. Joins all bridge threads — they exit via crossbeam disconnection
    ///
    /// Uses partial moves — IpcClient has no Drop impl (ClientGuard handles
    /// cancellation), so drop(send_state) before bridge join is legal.
    pub async fn shutdown(mut self) {
        tracing::debug!("client shutdown: sending GOODBYE");
        let _ = self.send_state.send_goodbye().await;

        if let Some(handle) = self.control_handle.take() {
            tracing::debug!("client shutdown: awaiting control loop (5s timeout)");
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(_) => {
                    tracing::debug!("client shutdown: control loop exited cleanly");
                }
                Err(_) => {
                    tracing::warn!("client shutdown: drain timeout — forcing close");
                    self.guard.cancel.cancel();
                }
            }
        }

        tracing::debug!("client shutdown: dropping send_state");
        drop(self.send_state);

        // Signal and close the shutdown eventfd immediately.
        // Mark as consumed (-1) so ClientGuard::drop doesn't double-signal.
        let efd = self.guard.shutdown_eventfd.swap(-1, std::sync::atomic::Ordering::AcqRel);
        if efd >= 0 {
            tracing::debug!("client shutdown: signaling read task eventfd");
            crate::v3::io::uring_read_task::signal_shutdown(efd);
            crate::v3::io::uring_read_task::close_shutdown_eventfd(efd);
        }

        // No bridge threads to join — OS threads send directly to tokio mpsc.

        // Drop owned_fd LAST — closes the socket fd after all uring
        // threads have exited. The threads held Copy'd RawFd values
        // that are only valid while this Arc keeps the fd alive.
        #[cfg(target_os = "linux")]
        {
            tracing::debug!("client shutdown: dropping owned_fd");
            drop(self.owned_fd.take());
        }

        tracing::debug!("client shutdown: complete");
    }
}

// No impl Drop for IpcClient — ClientGuard handles cancellation.
// This enables partial moves in shutdown().

impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient")
            .field("session_id", &self.session_id)
            .field("phase", &*self.phase.lock())
            .finish_non_exhaustive()
    }
}
