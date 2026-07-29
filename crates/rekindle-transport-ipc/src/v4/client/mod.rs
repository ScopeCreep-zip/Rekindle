//! IPC bus client — connects to a server, performs Noise IK handshake,
//! spawns per-connection lane tasks via `io::connection`, exposes typed
//! send/recv API.
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

use crate::v4::config::SessionConfig;
use crate::v4::io::connection::{self, ArenaBootstrap, ConnectionParams, ConnectionRuntime};
use crate::v4::io::lane_channels::RecvBufPool;
use crate::v4::io::lane_channels::WireBufPool;
use crate::v4::router::{ConnectionInfo, ConnectionPhase, FrameRouter};
use crate::v4::session::handshake::{dialler_handshake, HandshakeConfig, HandshakeError};
use crate::v4::session::SessionRole;
use crate::v4::socket::{self, PeerCredentials};
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::frame_class::FrameClass;
use crate::v4::wire::frame_kind::DatagramKind;
use crate::v4::wire::outbound::OutboundFrame;

use send::SendState;
use recv::InboundReceiver;

impl From<HandshakeError> for ClientError {
    fn from(e: HandshakeError) -> Self {
        Self::Handshake(format!("{e:?}"))
    }
}

// ── ClientRouter ────────────────────────────────────────────────

struct ClientRouter {
    inbound_tx: mpsc::Sender<InboundFrame>,
    pending_acks: Arc<Mutex<HashMap<uuid::Uuid, send::AckWaiter>>>,
    pending_bulk: Arc<Mutex<HashMap<u8, send::BulkWaiter>>>,
    pending_replies: Arc<Mutex<HashMap<uuid::Uuid, ReplySender>>>,
    #[cfg(target_os = "linux")]
    arena_write_handler: Arc<parking_lot::Mutex<Option<Box<dyn Fn(&crate::v4::router::ConnectionInfo, &crate::v4::streaming::shared_arena::SharedMemRef, &[u8]) + Send + Sync>>>>,
}

impl FrameRouter for ClientRouter {
    fn route_frame(&self, info: &ConnectionInfo, class: u8, kind: u8, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class, kind, payload: payload.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: None, correlation_id: None, sender_clearance: None,
            subscription_id: None, topic_hash: None, event_seq: None,
            status_phase: None, transfer_id: None, stream_id: None, chunk_index: None,
        });
    }

    fn on_request(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Request as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: Some(message_id), correlation_id: None,
            sender_clearance: Some(sender_clearance),
            subscription_id: None, topic_hash: None, event_seq: None,
            status_phase: None, transfer_id: None, stream_id: None, chunk_index: None,
        });
    }

    fn on_notify(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Notify as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: Some(message_id), correlation_id: None,
            sender_clearance: Some(sender_clearance),
            subscription_id: None, topic_hash: None, event_seq: None,
            status_phase: None, transfer_id: None, stream_id: None, chunk_index: None,
        });
    }

    fn on_publish(&self, info: &ConnectionInfo, subscription_id: uuid::Uuid, topic_hash: &[u8; 32], event_seq: u32, payload: &[u8]) {
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Publish as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: None, correlation_id: None, sender_clearance: None,
            subscription_id: Some(subscription_id), topic_hash: Some(*topic_hash),
            event_seq: Some(event_seq), status_phase: None, transfer_id: None,
            stream_id: None, chunk_index: None,
        });
    }

    fn on_reply(&self, info: &ConnectionInfo, reply_message_id: uuid::Uuid, correlation_id: uuid::Uuid, status_phase: u32, payload: &[u8]) {
        if let Some(reply_tx) = self.pending_replies.lock().remove(&correlation_id) {
            let _ = reply_tx.send(Ok(ReplyPayload { payload: payload.to_vec(), status_phase }));
            return;
        }
        if let Some(waiter) = self.pending_acks.lock().remove(&correlation_id) {
            let _ = waiter.tx.send(Ok(()));
        }
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Reply as u8,
            payload: payload.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: Some(reply_message_id), correlation_id: Some(correlation_id),
            sender_clearance: None, subscription_id: None, topic_hash: None,
            event_seq: None, status_phase: Some(status_phase), transfer_id: None,
            stream_id: None, chunk_index: None,
        });
    }

    fn on_reject(&self, info: &ConnectionInfo, rejected_message_id: uuid::Uuid, reason_code: u32, detail: &str) {
        if let Some(reply_tx) = self.pending_replies.lock().remove(&rejected_message_id) {
            let _ = reply_tx.send(Err(RequestReplyError::Rejected { reason_code, detail: detail.to_owned() }));
            self.pending_acks.lock().remove(&rejected_message_id);
            return;
        }
        if let Some(waiter) = self.pending_acks.lock().remove(&rejected_message_id) {
            let _ = waiter.tx.send(Err(SendError::Rejected { reason_code, detail: detail.to_owned() }));
        }
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Control,
            class: FrameClass::Datagram as u8, kind: DatagramKind::Reject as u8,
            payload: detail.as_bytes().to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: Some(rejected_message_id), correlation_id: None,
            sender_clearance: None, subscription_id: None, topic_hash: None,
            event_seq: None, status_phase: None, transfer_id: None,
            stream_id: None, chunk_index: None,
        });
    }

    fn on_bulk_complete(&self, _info: &ConnectionInfo, stream_id: u8, _transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32) {
        if let Some(waiter) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = waiter.tx.send(Ok(BulkDelivered {
                bytes_transferred: total_bytes,
                duration: waiter.started_at.elapsed(),
                chunks: total_chunks as u64,
            }));
        }
    }

    fn on_bulk_failed(&self, _info: &ConnectionInfo, stream_id: u8, _transfer_id: uuid::Uuid, reason: &str) {
        if let Some(waiter) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = waiter.tx.send(Err(BulkError::PeerRejected { reason: reason.to_owned() }));
        }
    }

    fn on_bulk_chunk(&self, _info: &ConnectionInfo, _stream_id: u8, _chunk_index: u32, _data: &[u8]) {}

    fn on_ack(&self, _info: &ConnectionInfo, message_ids: &[uuid::Uuid]) {
        let mut pending = self.pending_acks.lock();
        for id in message_ids {
            if let Some(waiter) = pending.remove(id) { let _ = waiter.tx.send(Ok(())); }
        }
    }

    fn on_connection_state_change(&self, info: &ConnectionInfo, old_phase: ConnectionPhase, new_phase: ConnectionPhase) {
        tracing::info!(conn_id = info.conn_id, %old_phase, %new_phase, "client state change");
    }

    fn on_arena_write(&self, info: &ConnectionInfo, shmref: &crate::v4::streaming::shared_arena::SharedMemRef, data: &[u8]) {
        #[cfg(target_os = "linux")]
        {
            let guard = self.arena_write_handler.lock();
            if let Some(ref handler) = *guard {
                handler(info, shmref, data);
                return;
            }
        }
        let _ = self.inbound_tx.try_send(InboundFrame {
            lane: crate::v4::wire::lane::Lane::Handoff, class: 0x05, kind: 0x01,
            payload: data.to_vec(),
            conn_id: info.conn_id, session_id: info.session_id, peer_id: info.peer_id,
            message_id: None, correlation_id: None, sender_clearance: None,
            subscription_id: None, topic_hash: None, event_seq: None, status_phase: None,
            transfer_id: None,
            stream_id: Some(shmref.arena_id), chunk_index: Some(shmref.slot as u32),
        });
    }

    fn on_dmabuf_ref(&self, _info: &ConnectionInfo, _dmabuf: &crate::v4::streaming::dmabuf::DmaBufRef, _payload_id: u8) {}
}

// ── ClientGuard ─────────────────────────────────────────────────

struct ClientGuard {
    cancel: tokio_util::sync::CancellationToken,
    shutdown_eventfd: std::sync::atomic::AtomicI32,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let efd = self.shutdown_eventfd.swap(-1, std::sync::atomic::Ordering::AcqRel);
        if efd >= 0 {
            crate::v4::io::uring_read_task::signal_shutdown(efd);
            crate::v4::io::uring_read_task::close_shutdown_eventfd(efd);
        }
        self.cancel.cancel();
    }
}

// ── IpcClient ───────────────────────────────────────────────────

pub struct IpcClient {
    send_state: SendState,
    recv_state: InboundReceiver,
    session_id: uuid::Uuid,
    agreed_clearance: Clearance,
    active_capabilities: CapabilityBits,
    phase: Arc<Mutex<ClientPhase>>,
    guard: ClientGuard,
    /// Connection runtime — owns uring thread handles, shutdown eventfd,
    /// socket fd. Consumed by shutdown() to join threads cleanly.
    runtime: Option<ConnectionRuntime>,
    #[cfg(target_os = "linux")]
    streaming_sender: Option<crate::v4::streaming::send::StreamingSender>,
    #[cfg(target_os = "linux")]
    arena_write_handler: Arc<parking_lot::Mutex<Option<Box<dyn Fn(&crate::v4::router::ConnectionInfo, &crate::v4::streaming::shared_arena::SharedMemRef, &[u8]) + Send + Sync>>>>,
}

impl IpcClient {
    pub async fn connect(
        path: &Path,
        server_public_key: &[u8; 32],
        client_keypair: &snow::Keypair,
        config: SessionConfig,
        handshake_config: HandshakeConfig,
    ) -> Result<Self, ClientError> {
        let mut stream = tokio::net::UnixStream::connect(path).await
            .map_err(ClientError::Connect)?;

        let peer_creds = socket::extract_ucred(&stream)
            .map_err(|e| ClientError::UcredFailed(format!("{e}")))?;
        let local_creds = PeerCredentials::local();
        let prologue = crate::v4::crypto::noise::build_prologue(
            local_creds.pid, local_creds.uid, peer_creds.pid, peer_creds.uid,
        ).map_err(|e| ClientError::PrologueFailed(format!("{e:?}")))?;

        #[cfg(unix)]
        socket::apply_socket_options(&stream, Some(262_144), Some(262_144));

        let hr = dialler_handshake(
            &mut stream, client_keypair, server_public_key,
            handshake_config, &prologue, Duration::from_secs(5),
        ).await?;

        let session_id = hr.session_id;
        let agreed_clearance = hr.agreed_clearance;
        let active_capabilities = hr.active_capabilities;
        let keys = hr.keys.clone();
        let remote_peer_id = hr.remote_peer_id;
        let encoder = Arc::new(hr.encoder);

        // Arena: receive sidechannel + fds from server
        #[cfg(target_os = "linux")]
        let (pending_arena_fds, client_sidechannel) = receive_arena_client(
            &stream, active_capabilities,
        );
        #[cfg(target_os = "linux")]
        let arena_bootstrap = ArenaBootstrap::Client {
            pending_fds: pending_arena_fds,
            sidechannel: client_sidechannel.clone(),
        };
        #[cfg(not(target_os = "linux"))]
        let arena_bootstrap = ArenaBootstrap::None;

        let std_stream = stream.into_std().map_err(ClientError::Connect)?;
        let raw_fd = { use std::os::unix::io::AsRawFd; std_stream.as_raw_fd() };
        let owned_fd = Arc::new(std_stream);

        let encrypt_pool = crate::v4::crypto::pool::build_encrypt_pool(
            config.encrypt_workers.unwrap_or(0),
        );
        let pool_capacity = encrypt_pool.current_num_threads() * 2;
        let counters = crate::v4::bulk::counters::BulkCounters::new();

        // Phase 1: prepare connection
        let mut prepared = connection::prepare_connection(
            ConnectionParams {
                raw_fd,
                owned_fd: Arc::clone(&owned_fd),
                encoder, decoder: hr.decoder,
                keys, handshake_hash: hr.handshake_hash,
                role: SessionRole::Dialler,
                agreed_aead: hr.agreed_aead,
                session_id, conn_id: 0,
                local_peer_id: hr.local_peer_id, remote_peer_id,
                agreed_clearance, active_capabilities,
                config: config.clone(),
                encrypt_pool: Arc::clone(&encrypt_pool),
                counters: Arc::clone(&counters),
                wire_pool: WireBufPool::new(pool_capacity),
                recv_pool: RecvBufPool::new(pool_capacity),
                thread_name_prefix: "rti-v4-client".into(),
            },
            arena_bootstrap,
        );

        // Construct client router + send/recv state using outbound_tx from prepared
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let (bulk_chunk_tx, bulk_chunk_rx) = mpsc::channel(64);

        let pending_acks = Arc::new(Mutex::new(HashMap::new()));
        let pending_bulk = Arc::new(Mutex::new(HashMap::new()));
        let pending_replies = Arc::new(Mutex::new(HashMap::new()));

        #[cfg(target_os = "linux")]
        let arena_write_handler: Arc<parking_lot::Mutex<Option<Box<dyn Fn(&crate::v4::router::ConnectionInfo, &crate::v4::streaming::shared_arena::SharedMemRef, &[u8]) + Send + Sync>>>> =
            Arc::new(parking_lot::Mutex::new(None));

        let client_router = Arc::new(ClientRouter {
            inbound_tx,
            pending_acks: Arc::clone(&pending_acks),
            pending_bulk: Arc::clone(&pending_bulk),
            pending_replies: Arc::clone(&pending_replies),
            #[cfg(target_os = "linux")]
            arena_write_handler: Arc::clone(&arena_write_handler),
        });

        let send_state = SendState {
            outbound_tx: prepared.outbound_tx.clone(),
            sequenced_tx: prepared.sequenced_tx.clone(),
            bulk_sender: prepared.bulk_sender.clone(),
            pending_acks, pending_bulk, pending_replies,
            phase: Arc::new(Mutex::new(ClientPhase::Ready)),
            agreed_clearance,
        };

        let cancelled_recv_streams = Arc::new(Mutex::new(HashSet::new()));
        let recv_state = InboundReceiver::new(
            inbound_rx, bulk_chunk_rx, Arc::clone(&cancelled_recv_streams),
        );

        let phase = Arc::clone(&send_state.phase);
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let shutdown_eventfd = prepared.shutdown_eventfd;

        // Bulk data bridge: Data lane → bulk_chunk_tx for client recv API
        let bulk_data_rx = prepared.bulk_data_rx.take()
            .expect("bulk_data_rx already taken");
        tokio::spawn(async move {
            let mut bulk_data_rx = bulk_data_rx;
            while let Some(signal) = bulk_data_rx.recv().await {
                let chunk = match signal {
                    crate::v4::io::control_loop::BulkDataSignal::Chunk { stream_id, chunk_index, data } => {
                        crate::v4::bulk::counters::DIAG_RECV_DELIVERED_BYTES
                            .fetch_add(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
                        crate::v4::bulk::counters::DIAG_RECV_DELIVERED_FRAMES
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        BulkChunk::from_plaintext(stream_id, chunk_index, data)
                    }
                    crate::v4::io::control_loop::BulkDataSignal::Complete { stream_id, total_chunks, .. } => {
                        BulkChunk::completion(stream_id, total_chunks)
                    }
                };
                if bulk_chunk_tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        // Phase 2: spawn lane tasks with the real router
        let runtime = connection::spawn_lanes(prepared, client_router);
        let lane_cancel = runtime.lane_cancel.clone();

        // Phase transition task — sets ClientPhase::Closed when lanes exit
        let phase_shutdown = Arc::clone(&phase);
        let shutdown_lane_cancel = lane_cancel.clone();
        let child_cancel = cancel_token.child_token();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = child_cancel.cancelled() => {}
                _ = shutdown_lane_cancel.cancelled() => {}
            }
            *phase_shutdown.lock() = ClientPhase::Closed;
        });

        #[cfg(target_os = "linux")]
        let streaming_sender = if active_capabilities.contains(CapabilityBits::SHARED_ARENA) {
            use crate::v4::streaming::shared_arena::SharedArena;
            use crate::v4::streaming::sidechannel;
            match SharedArena::create(config.arena_slot_size, config.arena_slot_count, config.arena_integrity_check) {
                Ok(client_arena) => {
                    if let Some(ref sc) = client_sidechannel {
                        match sidechannel::send_arena_fds(sc, client_arena.arena_fd(), client_arena.states_fd()) {
                            Ok(()) => {
                                let client_arena = Arc::new(client_arena);
                                let setup = crate::v4::codec::streaming::arena_setup::ArenaSetupPayload {
                                    slot_size: client_arena.slot_size() as u32,
                                    slot_count: client_arena.slot_count() as u16,
                                    integrity: if config.arena_integrity_check { 1 } else { 0 },
                                };
                                let setup_bytes = crate::v4::codec::streaming::arena_setup::encode(&setup);
                                let _ = send_state.outbound_tx.send(OutboundFrame::Handoff {
                                    kind: crate::v4::wire::frame_kind::HandoffKind::ArenaSetup,
                                    payload: setup_bytes.to_vec(),
                                });
                                Some(crate::v4::streaming::send::StreamingSender::new(
                                    Some(client_arena), 0, client_sidechannel.clone(),
                                    send_state.outbound_tx.clone(),
                                ))
                            }
                            Err(e) => { tracing::warn!(error = ?e, "client arena: fd send failed"); None }
                        }
                    } else { None }
                }
                Err(e) => { tracing::warn!(error = ?e, "client arena: create failed"); None }
            }
        } else { None };

        Ok(Self {
            send_state,
            recv_state,
            session_id,
            agreed_clearance,
            active_capabilities,
            phase,
            guard: ClientGuard {
                cancel: cancel_token,
                shutdown_eventfd: std::sync::atomic::AtomicI32::new(shutdown_eventfd),
            },
            runtime: Some(runtime),
            #[cfg(target_os = "linux")]
            streaming_sender,
            #[cfg(target_os = "linux")]
            arena_write_handler,
        })
    }

    pub fn session_id(&self) -> uuid::Uuid { self.session_id }
    pub fn agreed_clearance(&self) -> Clearance { self.agreed_clearance }
    pub fn active_capabilities(&self) -> CapabilityBits { self.active_capabilities }
    pub fn phase(&self) -> ClientPhase { *self.phase.lock() }

    #[cfg(target_os = "linux")]
    pub fn streaming_sender(&self) -> Option<&crate::v4::streaming::send::StreamingSender> {
        self.streaming_sender.as_ref()
    }

    /// Set a zero-copy callback for inbound streaming arena writes.
    /// The callback receives `&[u8]` directly from the mmap'd SharedArena.
    /// Zero copy. The transport sends SlotRelease after callback returns.
    /// The callback runs on the Handoff lane tokio task — do not block.
    /// Call this before any ArenaWrite frames arrive (immediately after connect).
    /// If not set, falls back to copying into InboundFrame.payload.
    #[cfg(target_os = "linux")]
    pub fn set_arena_write_handler<F>(&self, handler: F)
    where
        F: Fn(&crate::v4::router::ConnectionInfo, &crate::v4::streaming::shared_arena::SharedMemRef, &[u8]) + Send + Sync + 'static,
    {
        *self.arena_write_handler.lock() = Some(Box::new(handler));
    }

    pub async fn send_request(&self, payload: &[u8], ack_timeout: Duration) -> Result<SendDelivered, SendError> {
        self.send_state.send_request(payload, ack_timeout).await
    }
    pub async fn request_reply(&self, payload: &[u8], timeout: Duration) -> Result<ReplyPayload, RequestReplyError> {
        self.send_state.request_reply(payload, timeout).await
    }
    pub async fn send_notify(&self, payload: &[u8]) -> Result<(), ClientError> {
        self.send_state.send_notify(payload).await
    }
    pub async fn send_bulk(&self, stream_id: u8, payload: &[u8], ack_timeout: Duration) -> Result<BulkDelivered, BulkError> {
        self.send_state.send_bulk(stream_id, payload, ack_timeout).await
    }
    pub async fn rotate_keys(&self, timeout: Duration) -> Result<(), BulkError> {
        self.send_state.rotate_keys(timeout).await
            .map_err(|e| BulkError::PeerRejected { reason: format!("{e:?}") })
    }
    pub async fn cancel_bulk(&self, stream_id: u8) {
        self.send_state.cancel_bulk(stream_id).await;
    }
    pub async fn recv(&self) -> Option<InboundFrame> { self.recv_state.recv().await }
    pub async fn recv_bulk_chunk(&self) -> Option<BulkChunk> { self.recv_state.recv_bulk_chunk().await }
    pub async fn recv_bulk(&self) -> Option<(u8, Vec<u8>)> { self.recv_state.recv_bulk().await }
    pub fn cancel_recv_bulk(&self, stream_id: u8) { self.recv_state.cancel_recv_bulk(stream_id); }

    pub async fn send_raw_outbound(&self, frame: OutboundFrame) -> Result<(), ClientError> {
        self.send_state.outbound_tx.send(frame)
            .map_err(|e| ClientError::Send(format!("{e:?}")))
    }

    pub async fn shutdown(mut self) {
        tracing::info!(session_id = %self.session_id, "IpcClient::shutdown: sending GOODBYE");
        let goodbye_result = self.send_state.send_goodbye().await;
        tracing::debug!(
            session_id = %self.session_id,
            goodbye_ok = goodbye_result.is_ok(),
            "IpcClient::shutdown: GOODBYE sent, awaiting lane drain"
        );

        // Wait for lane tasks to finish or timeout
        if let Some(ref runtime) = self.runtime {
            let lc = runtime.lane_cancel.clone();
            tokio::select! {
                _ = lc.cancelled() => {
                    tracing::debug!(session_id = %self.session_id, "IpcClient::shutdown: lanes cancelled cleanly");
                }
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    tracing::warn!(session_id = %self.session_id, "client shutdown: drain timeout — forcing close");
                    self.guard.cancel.cancel();
                }
            }
        }

        drop(self.send_state);
        tracing::debug!(session_id = %self.session_id, "IpcClient::shutdown: send_state dropped");

        // Mark eventfd as consumed so ClientGuard::drop doesn't double-signal
        let efd = self.guard.shutdown_eventfd.swap(-1, std::sync::atomic::Ordering::AcqRel);
        if efd >= 0 {
            crate::v4::io::uring_read_task::signal_shutdown(efd);
            crate::v4::io::uring_read_task::close_shutdown_eventfd(efd);
        }

        // Join uring threads and close socket fd
        if let Some(runtime) = self.runtime.take() {
            tracing::debug!(session_id = %self.session_id, "IpcClient::shutdown: joining uring threads");
            connection::shutdown_connection(runtime);
        }
        tracing::info!(session_id = %self.session_id, "IpcClient::shutdown: complete");
    }
}

impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient")
            .field("session_id", &self.session_id)
            .field("phase", &*self.phase.lock())
            .finish_non_exhaustive()
    }
}

// ── Arena receive helper (client side) ──────────────────────────

#[cfg(target_os = "linux")]
fn receive_arena_client(
    stream: &tokio::net::UnixStream,
    active_capabilities: CapabilityBits,
) -> (
    Option<(std::os::unix::io::OwnedFd, std::os::unix::io::OwnedFd)>,
    Option<Arc<crate::v4::streaming::sidechannel::SideChannel>>,
) {
    if !active_capabilities.contains(CapabilityBits::SHARED_ARENA) {
        return (None, None);
    }
    use std::os::unix::io::AsRawFd;
    use crate::v4::streaming::sidechannel;

    match sidechannel::recv_fd_from_stream(stream.as_raw_fd()) {
        Ok(sc_fd) => {
            let sc = sidechannel::SideChannel::from_raw_fd(sc_fd);
            match sidechannel::recv_arena_fds(&sc) {
                Ok((arena_fd, states_fd)) => {
                    (Some((arena_fd, states_fd)), Some(Arc::new(sc)))
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "arena: fd recv failed");
                    (None, None)
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = ?e, "sidechannel: recv failed");
            (None, None)
        }
    }
}
