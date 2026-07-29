//! Two-phase connection setup — shared infrastructure between server and client.
//!
//! Phase 1: `prepare_connection(params)` — creates channels, per-lane state,
//! shared state, BulkSender, BulkReceiver, uring threads. Returns
//! `ConnectionPrepared` with outbound_tx and bulk_sender for the caller
//! to construct a router.
//!
//! Phase 2: `spawn_lanes(prepared, router)` — spawns 4 lane tasks + 1 audit
//! merge + 1 bulk data bridge. Returns `ConnectionRuntime` for lifetime
//! management.
//!
//! The two-phase design breaks the circular dependency: the router needs
//! outbound_tx (created in phase 1), and the lane tasks need the router
//! (provided in phase 2).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::v4::audit::chain::AuditChain;
use crate::v4::audit::checkpoint::CheckpointTracker;
use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::bulk::counters::BulkCounters;
use crate::v4::bulk::send::BulkSender;
use crate::v4::config::SessionConfig;
use crate::v4::crypto::keys::DerivedKeys;
use crate::v4::dedup::cache::{ReceiverCache, SenderCache};
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::io::control_loop::audit_merge::{self, AuditLinkDirection};
use crate::v4::io::control_loop::audit_reorder::AuditReorderBuffer;
use crate::v4::io::control_loop::fin::PendingFin;
use crate::v4::io::control_loop::lane::data::DataRevocation;
use crate::v4::io::control_loop::lane::state::{
    AuditState, ControlState, DataState, HandoffState,
};
use crate::v4::io::control_loop::lane::FrameLoop;
use crate::v4::io::control_loop::shared_state::{
    CipherKeySet, SessionStateHandle, SharedAuditLinks, SharedSessionState,
};
use crate::v4::io::control_loop::{self, BulkDataSignal, WriteError};
use crate::v4::io::encode::{EpochKeys, FrameEncoder};
use crate::v4::io::epoch_signal::EpochSignal;
use crate::v4::io::lane_channels::{LaneChannels, RecvBufPool, WireBufPool};
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::session::rotation::RotationCoordinator;
use crate::v4::session::SessionRole;
use crate::v4::stream::flow_control::BackpressureState;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::resume::ResumeRegistry;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::outbound::{OutboundFrame, SequencedOutbound};

#[cfg(target_os = "linux")]
use crate::v4::streaming::shared_arena::SharedArena;
#[cfg(target_os = "linux")]
use crate::v4::streaming::sidechannel::SideChannel;

// ── Connection parameters ───────────────────────────────────────

pub struct ConnectionParams {
    pub raw_fd: i32,
    pub owned_fd: Arc<std::os::unix::net::UnixStream>,
    pub encoder: Arc<FrameEncoder>,
    pub decoder: crate::v4::io::decode::FrameDecoder,
    pub keys: DerivedKeys,
    pub handshake_hash: [u8; 32],
    pub role: SessionRole,
    pub agreed_aead: u8,
    pub session_id: uuid::Uuid,
    pub conn_id: u64,
    pub local_peer_id: [u8; 32],
    pub remote_peer_id: [u8; 32],
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
    pub config: SessionConfig,
    pub encrypt_pool: Arc<rayon::ThreadPool>,
    pub counters: Arc<BulkCounters>,
    pub wire_pool: WireBufPool,
    pub recv_pool: RecvBufPool,
    pub thread_name_prefix: String,
}

pub enum ArenaBootstrap {
    #[cfg(target_os = "linux")]
    Server {
        arenas: Vec<Arc<SharedArena>>,
        sidechannel: Option<Arc<SideChannel>>,
        initial_outbound: Vec<OutboundFrame>,
    },
    #[cfg(target_os = "linux")]
    Client {
        pending_fds: Option<(std::os::unix::io::OwnedFd, std::os::unix::io::OwnedFd)>,
        sidechannel: Option<Arc<SideChannel>>,
    },
    None,
}

// ── Phase 1 output ──────────────────────────────────────────────

pub struct ConnectionPrepared {
    pub outbound_tx: mpsc::UnboundedSender<OutboundFrame>,
    pub sequenced_tx: mpsc::Sender<SequencedOutbound>,
    pub bulk_sender: BulkSender,
    pub shutdown_eventfd: i32,
    pub read_handle: std::thread::JoinHandle<()>,
    pub write_handle: std::thread::JoinHandle<()>,
    pub owned_fd: Arc<std::os::unix::net::UnixStream>,
    pub conn_id: u64,
    pub session_id: uuid::Uuid,
    pub remote_peer_id: [u8; 32],
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
    // Internal — consumed by spawn_lanes
    outbound_rx: mpsc::UnboundedReceiver<OutboundFrame>,
    sequenced_rx: mpsc::Receiver<SequencedOutbound>,
    write_error_rx: mpsc::Receiver<WriteError>,
    pending_fin_rx: mpsc::Receiver<PendingFin>,
    revocation_rx: mpsc::Receiver<DataRevocation>,
    audit_merge_tx: mpsc::Sender<AuditLinkDirection>,
    audit_merge_rx: mpsc::Receiver<AuditLinkDirection>,
    lane_channels: LaneChannels,
    inbound_lane_receivers: control_loop::LaneInboundReceivers,
    encoder: Arc<FrameEncoder>,
    counters: Arc<BulkCounters>,
    shared: SessionStateHandle,
    audit_links: Arc<SharedAuditLinks>,
    pub connection_info: ConnectionInfo,
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    outbound_chain: AuditChain,
    inbound_chain: AuditChain,
    outbound_audit_queue: crate::v4::bulk::AuditQueue,
    outbound_audit_wake: rekindle_transport_buff::adapters::tokio::TokioWake,
    recv_result_queue: Arc<rekindle_transport_buff::DispatchQueue<
        crate::v4::bulk::recv::BulkDecryptResult,
        rekindle_transport_buff::adapters::tokio::TokioWake,
    >>,
    pub bulk_data_rx: Option<mpsc::Receiver<BulkDataSignal>>,
    bulk_data_tx: mpsc::Sender<BulkDataSignal>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    control_state: ControlState,
    data_state: DataState,
    handoff_state: HandoffState,
    audit_state: AuditState,
    config: Arc<SessionConfig>,
}

// ── Phase 2 output ──────────────────────────────────────────────

pub struct ConnectionRuntime {
    pub outbound_tx: mpsc::UnboundedSender<OutboundFrame>,
    pub sequenced_tx: mpsc::Sender<SequencedOutbound>,
    pub bulk_sender: BulkSender,
    pub lane_cancel: CancellationToken,
    pub shutdown_eventfd: i32,
    pub read_handle: std::thread::JoinHandle<()>,
    pub write_handle: std::thread::JoinHandle<()>,
    pub owned_fd: Arc<std::os::unix::net::UnixStream>,
}

// ── Phase 1: prepare ────────────────────────────────────────────

pub fn prepare_connection(
    params: ConnectionParams,
    arena: ArenaBootstrap,
) -> ConnectionPrepared {
    let config = Arc::new(params.config);
    let channel_capacity = 256usize;

    // Channels
    let (lane_channels, bulk_wire_sender, lane_receivers) = LaneChannels::new(channel_capacity);
    let (inbound_lane_senders, inbound_lane_receivers) =
        control_loop::create_lane_inbound_channels();
    let (write_error_tx, write_error_rx) = mpsc::channel::<WriteError>(4);
    let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundFrame>();
    let (sequenced_tx, sequenced_rx) = mpsc::channel::<SequencedOutbound>(64);
    let (pending_fin_tx, pending_fin_rx) = mpsc::channel::<PendingFin>(64);
    let (outbound_audit_queue, outbound_audit_wake) =
        crate::v4::bulk::new_audit_queue(channel_capacity);
    tracing::debug!(
        conn_id = params.conn_id,
        channel_capacity,
        "connection: created outbound_audit_queue (inbound audit links flow through DecryptedChunk.link_input)"
    );
    let (bulk_data_tx, bulk_data_rx) = mpsc::channel::<BulkDataSignal>(64);
    let (audit_merge_tx, audit_merge_rx) =
        mpsc::channel::<AuditLinkDirection>(channel_capacity);
    let (revocation_tx, revocation_rx) =
        mpsc::channel::<DataRevocation>(64);

    // BulkSender
    let bulk_crossbeam_tx = bulk_wire_sender.into_crossbeam_sender();
    tracing::debug!(
        conn_id = params.conn_id,
        "connection: creating BulkSender with outbound_audit_queue (for sender-side audit links)"
    );
    let bulk_sender = BulkSender::new(
        Arc::clone(&params.encoder),
        Arc::clone(&params.encrypt_pool),
        bulk_crossbeam_tx,
        Arc::clone(&outbound_audit_queue),
        sequenced_tx.clone(),
        pending_fin_tx,
        params.wire_pool.clone(),
    );

    // BulkReceiver
    let (inbound_envelope_key, inbound_header_key, inbound_stream_key, inbound_direction_id) =
        match params.role {
            SessionRole::Listener => (
                params.keys.envelope_d2l, params.keys.header_d2l,
                &params.keys.stream_d2l, crate::v4::wire::constants::DIRECTION_ID_D2L,
            ),
            SessionRole::Dialler => (
                params.keys.envelope_l2d, params.keys.header_l2d,
                &params.keys.stream_l2d, crate::v4::wire::constants::DIRECTION_ID_L2D,
            ),
        };
    let inbound_cipher = crate::v4::session::handshake::build_bulk_cipher(
        params.agreed_aead, inbound_stream_key, inbound_direction_id,
    ).expect("inbound cipher init");
    let recv_result_wake = rekindle_transport_buff::adapters::tokio::TokioWake::new();
    let recv_result_queue = Arc::new(
        rekindle_transport_buff::DispatchQueue::new(64, recv_result_wake.clone()),
    );
    tracing::debug!(
        conn_id = params.conn_id,
        "connection: creating BulkReceiver (inbound audit links via DecryptedChunk.link_input)"
    );
    let bulk_receiver = crate::v4::bulk::recv::BulkReceiver::new(
        inbound_envelope_key, inbound_header_key,
        Arc::new(inbound_cipher),
        Arc::clone(&params.encrypt_pool),
        Arc::clone(&recv_result_queue),
        params.recv_pool, params.wire_pool,
    );
    let bulk_threshold = config.bulk_decrypt_threshold.unwrap_or(65536);
    let credit_guard = Arc::new(
        rekindle_transport_buff::CreditGuard::new(config.max_pending_bytes_per_session),
    );

    // Shared state
    let (out_env_key, out_hdr_key, out_stream_key, out_dir_id) = match params.role {
        SessionRole::Listener => (
            params.keys.envelope_l2d, params.keys.header_l2d,
            &params.keys.stream_l2d, crate::v4::wire::constants::DIRECTION_ID_L2D,
        ),
        SessionRole::Dialler => (
            params.keys.envelope_d2l, params.keys.header_d2l,
            &params.keys.stream_d2l, crate::v4::wire::constants::DIRECTION_ID_D2L,
        ),
    };
    let initial_cipher = CipherKeySet {
        epoch: 0,
        keys: EpochKeys {
            envelope_key: out_env_key, header_key: out_hdr_key,
            cipher: crate::v4::codec::aead::FrameCipher::aes256gcm(out_stream_key, out_dir_id)
                .expect("cipher init"),
        },
    };
    let shared: SessionStateHandle = Arc::new(parking_lot::RwLock::new(SharedSessionState {
        cipher_keys: Arc::new(initial_cipher),
        shutting_down: false,
        session_state: crate::v4::session::state::SessionState::Established,
        capabilities: params.active_capabilities,
    }));

    let (outbound_audit_key, inbound_audit_key) = match params.role {
        SessionRole::Dialler => (params.keys.audit_d2l, params.keys.audit_l2d),
        SessionRole::Listener => (params.keys.audit_l2d, params.keys.audit_d2l),
    };
    let outbound_chain = AuditChain::new(outbound_audit_key, params.handshake_hash);
    let inbound_chain = AuditChain::new(inbound_audit_key, params.handshake_hash);
    let audit_links = Arc::new(SharedAuditLinks::new(params.handshake_hash));

    let connection_info = ConnectionInfo {
        conn_id: params.conn_id, session_id: params.session_id,
        peer_id: params.remote_peer_id, clearance: params.agreed_clearance,
        capabilities: params.active_capabilities,
    };

    let epoch_signal = Arc::new(EpochSignal::new());
    let epoch_signal_for_read = Arc::clone(&epoch_signal);
    let encoder_for_read = Arc::clone(&params.encoder);
    let last_activity_ns = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let shutdown_eventfd = crate::v4::io::uring_read_task::create_shutdown_eventfd();

    // Per-lane state — all fields aligned with state.rs definitions
    let control_state = ControlState {
        rotation: RotationCoordinator::new(params.keys.clone(), 0),
        pending_requests: PendingRequestTracker::new(config.max_pending_requests),
        subscriptions: SubscriptionRegistry::new(config.max_subscriptions),
        send_seq: 0, // overwritten from encoder.current_session_seq() before GOODBYE
        last_ping_nonce: None, previous_ping_nonce: None,
        last_pong_received: None, heartbeat_miss_count: 0,
        remote_last_seen_our_seq: 0, peer_final_session_seq: None,
        local_goodbye_sent: false,
        quiescence_deadline: None, rotation_deadline: None, drain_deadline: None,
        pending_rotation_confirm: None,
        keys: params.keys.clone(), role: params.role, agreed_aead: params.agreed_aead,
        agreed_clearance: params.agreed_clearance,
        active_capabilities: params.active_capabilities,
        signal_epoch: 0, epoch_signal: Arc::clone(&epoch_signal),
        audit_links: Arc::clone(&audit_links),
        revocation_tx,
        audit_merge_tx: audit_merge_tx.clone(),
        config: Arc::clone(&config),
    };

    // Shared retention buffer — accessed by Data lane (sack remove)
    // and Audit lane (gap read, replay store). One allocation, two Arc clones.
    let retention = Arc::new(parking_lot::Mutex::new(
        RetentionBuffer::new(config.retention_config.clone()),
    ));

    let data_state = DataState {
        stream_registry: StreamRegistry::new(),
        reassemblers: HashMap::new(), stream_credits: HashMap::new(),
        lane_credit_bytes: HashMap::new(), backpressure: BackpressureState::new(),
        chunk_to_seq: HashMap::new(), resume_registry: ResumeRegistry::new(),
        sender_cache: SenderCache::new(config.sender_cache_config.clone()),
        receiver_cache: ReceiverCache::new(config.receiver_cache_config.clone()),
        pending_fin_verify: HashMap::new(), early_bulk_chunks: HashMap::new(),
        pending_bulk_deliveries: Vec::new(), pending_bulk_completions: Vec::new(),
        pending_fins: HashMap::new(),
        pending_outbound: Vec::new(),
        retention: Arc::clone(&retention),
        agreed_clearance: params.agreed_clearance,
        active_capabilities: params.active_capabilities,
        config: Arc::clone(&config),
    };

    let mut handoff_state = HandoffState {
        #[cfg(target_os = "linux")] arenas: Vec::new(),
        #[cfg(target_os = "linux")] sidechannel: None,
        #[cfg(target_os = "linux")] pending_arena_fds: None,
        conn_id: params.conn_id,
    };
    #[cfg(target_os = "linux")]
    match arena {
        ArenaBootstrap::Server { arenas, sidechannel, .. } => {
            handoff_state.arenas = arenas;
            handoff_state.sidechannel = sidechannel;
        }
        ArenaBootstrap::Client { pending_fds, sidechannel } => {
            handoff_state.pending_arena_fds = pending_fds;
            handoff_state.sidechannel = sidechannel;
        }
        ArenaBootstrap::None => {}
    }
    #[cfg(not(target_os = "linux"))]
    debug_assert!(
        matches!(arena, ArenaBootstrap::None),
        "ArenaBootstrap::Server or ArenaBootstrap::Client passed on non-Linux — \
         shared memory streaming requires Linux memfd + mmap. \
         Arena fds were allocated and will be leaked."
    );

    let audit_state = AuditState {
        retention,
        verified_proofs: HashMap::new(),
        audit_links: Arc::clone(&audit_links),
        audit_merge_tx: audit_merge_tx.clone(),
    };

    // Spawn uring threads
    let conn_id = params.conn_id;
    let raw_fd = params.raw_fd;

    let write_counters = Arc::clone(&params.counters);
    let write_fd_hold = Arc::clone(&params.owned_fd);
    let write_handle = std::thread::Builder::new()
        .name(format!("{}-write-{conn_id}", params.thread_name_prefix))
        .spawn(move || {
            crate::v4::io::uring_write_task::run(
                raw_fd,
                lane_receivers.control_rx, lane_receivers.audit_rx,
                lane_receivers.handoff_rx, lane_receivers.data_rx,
                lane_receivers.bulk_rx, write_counters, write_error_tx,
            );
            drop(write_fd_hold);
        })
        .expect("failed to spawn uring write task");

    let read_counters = Arc::clone(&params.counters);
    let read_fd_hold = Arc::clone(&params.owned_fd);
    let read_credit_guard = Arc::clone(&credit_guard);
    let read_handle = std::thread::Builder::new()
        .name(format!("{}-read-{conn_id}", params.thread_name_prefix))
        .spawn(move || {
            crate::v4::io::uring_read_task::run(
                raw_fd, params.decoder, inbound_lane_senders,
                bulk_receiver, bulk_threshold, read_counters,
                read_credit_guard, shutdown_eventfd,
                epoch_signal_for_read, encoder_for_read,
            );
            drop(read_fd_hold);
        })
        .expect("failed to spawn uring read task");

    ConnectionPrepared {
        bulk_data_rx: Some(bulk_data_rx),
        outbound_tx: outbound_tx.clone(),
        sequenced_tx: sequenced_tx.clone(),
        bulk_sender,
        shutdown_eventfd,
        read_handle, write_handle,
        owned_fd: params.owned_fd,
        conn_id: params.conn_id,
        session_id: params.session_id,
        remote_peer_id: params.remote_peer_id,
        agreed_clearance: params.agreed_clearance,
        active_capabilities: params.active_capabilities,
        outbound_rx, sequenced_rx, write_error_rx, pending_fin_rx,
        revocation_rx,
        audit_merge_tx, audit_merge_rx,
        lane_channels, inbound_lane_receivers,
        encoder: params.encoder,
        counters: params.counters,
        shared, audit_links, connection_info,
        last_activity_ns,
        outbound_chain, inbound_chain,
        outbound_audit_queue, outbound_audit_wake,
        recv_result_queue, bulk_data_tx, credit_guard,
        control_state, data_state, handoff_state, audit_state,
        config,
    }
}

// ── Phase 2: spawn lanes ────────────────────────────────────────

pub fn spawn_lanes(
    prepared: ConnectionPrepared,
    router: Arc<dyn FrameRouter>,
) -> ConnectionRuntime {
    let lane_cancel = CancellationToken::new();

    // Destructure prepared — each field is moved exactly once.
    // This prevents use-after-move when spawning multiple async tasks.
    let ConnectionPrepared {
        outbound_tx, sequenced_tx, bulk_sender,
        shutdown_eventfd, read_handle, write_handle, owned_fd,
        conn_id: _, session_id: _, remote_peer_id: _,
        agreed_clearance: _, active_capabilities: _,
        outbound_rx, sequenced_rx, write_error_rx,
        pending_fin_rx, revocation_rx,
        audit_merge_tx, audit_merge_rx,
        lane_channels, inbound_lane_receivers,
        encoder, counters, shared, audit_links,
        connection_info, last_activity_ns,
        outbound_chain, inbound_chain,
        outbound_audit_queue, outbound_audit_wake,
        recv_result_queue, bulk_data_rx: _, bulk_data_tx, credit_guard,
        control_state, data_state, handoff_state, audit_state,
        config,
    } = prepared;

    let control_loop::LaneInboundReceivers {
        control_rx, data_rx, audit_rx, handoff_rx,
    } = inbound_lane_receivers;

    // All lane tasks share the same retention buffer via Arc<Mutex>.
    let shared_retention = Arc::clone(&data_state.retention);

    let make_frame_loop = |shared: &SessionStateHandle,
                           audit_tx: &mpsc::Sender<AuditLinkDirection>,
                           lc: &LaneChannels,
                           enc: &Arc<FrameEncoder>,
                           ret: &Arc<parking_lot::Mutex<RetentionBuffer>>| {
        FrameLoop::new(Arc::clone(shared), audit_tx.clone(), lc.clone(), Arc::clone(enc), Arc::clone(ret))
    };

    // Handoff lane
    {
        let fl = make_frame_loop(&shared, &audit_merge_tx, &lane_channels, &encoder, &shared_retention);
        let r = Arc::clone(&router);
        let i = connection_info.clone();
        let c = lane_cancel.clone();
        tokio::spawn(async move {
            crate::v4::io::control_loop::lane::handoff::run(
                handoff_rx, handoff_state, fl, r, i,
            ).await;
            c.cancel();
        });
    }

    // Audit lane
    {
        let fl = make_frame_loop(&shared, &audit_merge_tx, &lane_channels, &encoder, &shared_retention);
        let c = lane_cancel.clone();
        tokio::spawn(async move {
            crate::v4::io::control_loop::lane::audit::run(
                audit_rx, audit_state, fl,
            ).await;
            c.cancel();
        });
    }

    // Data lane
    {
        let fl = make_frame_loop(&shared, &audit_merge_tx, &lane_channels, &encoder, &shared_retention);
        let r = Arc::clone(&router);
        let i = connection_info.clone();
        let al = Arc::clone(&audit_links);
        let c = lane_cancel.clone();
        tracing::debug!(
            "spawn_lanes: Data lane receives outbound_audit_queue (BulkSender pushes here) \
             and recv_result_queue (BulkReceiver pushes here)"
        );
        tokio::spawn(async move {
            crate::v4::io::control_loop::lane::data::run(
                data_rx, data_state, fl, r, i,
                recv_result_queue, outbound_audit_queue, outbound_audit_wake,
                pending_fin_rx, revocation_rx,
                bulk_data_tx, credit_guard, al,
            ).await;
            c.cancel();
        });
    }

    // Control lane
    {
        let fl = make_frame_loop(&shared, &audit_merge_tx, &lane_channels, &encoder, &shared_retention);
        let r = Arc::clone(&router);
        let i = connection_info.clone();
        let la = Arc::clone(&last_activity_ns);
        let cnt = Arc::clone(&counters);
        let c = lane_cancel.clone();
        tokio::spawn(async move {
            crate::v4::io::control_loop::lane::control::run(
                control_rx, control_state, fl, r, i,
                write_error_rx, outbound_rx, sequenced_rx, cnt, la,
            ).await;
            c.cancel();
        });
    }

    // Audit merge
    {
        let lc = lane_channels;
        let e = encoder;
        let outbound_cp = CheckpointTracker::new(config.checkpoint_config);
        let inbound_cp = CheckpointTracker::new(config.checkpoint_config);
        let max_stall = std::time::Duration::from_millis(
            config.max_reorder_stall_ms.unwrap_or(500),
        );
        let al = audit_links;
        let c = lane_cancel.clone();
        tokio::spawn(async move {
            audit_merge::run(
                audit_merge_rx, outbound_chain, inbound_chain,
                AuditReorderBuffer::with_label("audit-merge-outbound", max_stall),
                AuditReorderBuffer::with_label("audit-merge-inbound", max_stall),
                outbound_cp, inbound_cp, lc, e, al,
            ).await;
            c.cancel();
        });
    }

    ConnectionRuntime {
        outbound_tx, sequenced_tx, bulk_sender,
        lane_cancel, shutdown_eventfd,
        read_handle, write_handle, owned_fd,
    }
}

/// Graceful shutdown — signal read task, join threads, close fd.
pub fn shutdown_connection(runtime: ConnectionRuntime) {
    crate::v4::io::uring_read_task::signal_shutdown(runtime.shutdown_eventfd);
    let _ = runtime.read_handle.join();
    crate::v4::io::uring_read_task::close_shutdown_eventfd(runtime.shutdown_eventfd);
    let _ = runtime.write_handle.join();
    drop(runtime.owned_fd);
}
