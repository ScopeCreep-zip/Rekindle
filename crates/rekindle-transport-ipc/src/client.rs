//! IPC bus client — connects to the server, sends/receives frames
//! over an encrypted Unix domain socket.
//!
//! Every send operation returns a future with a deterministic outcome:
//! - `send_frame` -> `Future<SendOutcome>` (Delivered/AckTimeout/WriteFailed)
//! - `send_bulk` -> `Future<BulkOutcome>` (Delivered/AckTimeout/WriteFailed/IntegrityFailed/...)
//!
//! No fire-and-forget. No silent drops. Every frame gets an outcome.
//!
//! Bridge-free: rayon encrypt workers call `blocking_send` directly into
//! the IO task's bulk channel. No OS thread per client.
//!
//! write_all is NOT cancellation-safe inside select!. All writes happen
//! AFTER select! resolves, outside the cancellation window.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use serde::Serialize;

use crate::bulk;
use crate::config::IpcConfig;
use crate::envelope::{Message, MessageContext, SecurityLevel};
use crate::error::{IpcError, IpcResult};
use crate::frame::codec::encode_frame;
use crate::frame::lane::{self, LANE_CONTROL};
use crate::noise;
use crate::socket::{extract_ucred, PeerCredentials};
use crate::transport_frame::*;

/// Bulk data lane byte. Named constant — NEVER derived from payload content.
const LANE_BULK: u8 = 0x01;

/// Shared rayon thread pools for multi-client processes.
///
/// Without sharing, N clients create N×2 pools (encrypt + decrypt) with M workers
/// each = 2×N×M OS threads. On a 4-core machine with 8 clients: 64 threads
/// competing for 8 hardware threads.
///
/// With sharing: 1 encrypt pool + 1 decrypt pool = 2×M threads total.
/// Pass the same `SharedPools` to all `IpcClient::connect` calls.
pub struct SharedPools {
    pub encrypt_pool: Arc<rayon::ThreadPool>,
    pub decrypt_pool: Arc<rayon::ThreadPool>,
}

impl SharedPools {
    pub fn new(config: &IpcConfig) -> Self {
        Self {
            encrypt_pool: bulk::encrypt::build_encrypt_pool(config.encrypt_workers),
            decrypt_pool: bulk::encrypt::build_encrypt_pool(config.encrypt_workers),
        }
    }
}

struct AckWaiter {
    tx: oneshot::Sender<()>,
}

struct BulkWaiter {
    tx: oneshot::Sender<BulkOutcome>,
    started_at: Instant,
}

/// A single bulk data chunk received from the server.
///
/// Carries the `ZeroizingBuf` by move — zero copies from decrypt through
/// delivery. The buffer is zeroized on drop (volatile writes, full capacity).
/// The `MemoryReservation` holds the GlobalMemoryGuard slot until the
/// application drops the chunk — correct memory accounting.
pub struct BulkChunk {
    pub stream_id: u8,
    pub chunk_seq: u32,
    pub data: bulk::ZeroizingBuf,
    pub is_last: bool,
    /// RAII global memory reservation. Released when the application drops this chunk.
    pub(crate) reservation: Option<crate::backpressure::MemoryReservation>,
    /// RAII per-connection memory reservation.
    pub(crate) per_conn_reservation: Option<crate::backpressure::MemoryReservation>,
}

impl std::fmt::Debug for BulkChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BulkChunk")
            .field("stream_id", &self.stream_id)
            .field("chunk_seq", &self.chunk_seq)
            .field("data_len", &self.data.len())
            .field("is_last", &self.is_last)
            .field("has_reservation", &self.reservation.is_some())
            .field("has_per_conn_reservation", &self.per_conn_reservation.is_some())
            .finish()
    }
}

/// The IPC bus client.
pub struct IpcClient {
    sender_id: Uuid,
    msg_ctx: MessageContext,
    app_tx: mpsc::Sender<(u64, Vec<u8>, oneshot::Sender<()>)>,
    transport_tx: mpsc::Sender<Vec<u8>>,
    inbound_rx: mpsc::Receiver<bytes::Bytes>,
    /// Bulk chunk receiver. Delivers ZeroizingBuf by move — zero copies.
    /// Used by both recv_bulk_chunk() (streaming) and recv_bulk() (buffered wrapper).
    bulk_chunk_rx: mpsc::Receiver<BulkChunk>,
    /// Buffer for chunks from non-target streams during recv_bulk().
    /// VecDeque preserves insertion order — FIFO delivery guaranteed.
    buffered_chunks: VecDeque<BulkChunk>,
    epoch: Instant,
    io_handle: tokio::task::JoinHandle<()>,
    inbound_handle: tokio::task::JoinHandle<()>,
    bulk_cipher: Option<Arc<bulk::BulkCipher>>,
    bulk_out_tx: mpsc::Sender<Vec<u8>>,
    encrypt_pool: Arc<rayon::ThreadPool>,
    send_buf_pool: Arc<bulk::BufferPool>,
    send_nonce_ctr: Arc<bulk::NonceCounter>,
    next_seq: AtomicU64,
    pending_bulk: Arc<parking_lot::Mutex<HashMap<u8, BulkWaiter>>>,
    phase: Arc<parking_lot::Mutex<ConnectionPhase>>,
    bulk_counters: Arc<bulk::BulkCounters>,
    /// Cancel signal for client-side recv reassemblers. Carries stream_id.
    cancel_recv_tx: mpsc::Sender<u8>,
    /// Streams cancelled by cancel_recv_bulk(). Shared between cancel_recv_bulk()
    /// (writer) and recv_bulk()/recv_bulk_chunk() (reader). The reassembly task
    /// also reads this to drop chunks before they reach bulk_chunk_tx.
    /// Single source of truth for cancellation state.
    cancelled_recv_streams: Arc<parking_lot::Mutex<std::collections::HashSet<u8>>>,
    /// Config max_frame_size for pre-send enforcement.
    /// Checked in send_frame BEFORE the frame enters the write task.
    /// The NoiseWriter also checks — but a rejection there kills the
    /// connection. Checking here rejects the single frame gracefully.
    max_frame_size: u32,
}

impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient")
            .field("sender_id", &self.sender_id)
            .field("phase", &*self.phase.lock())
            .field("next_seq", &self.next_seq.load(std::sync::atomic::Ordering::Relaxed))
            .field("has_bulk_cipher", &self.bulk_cipher.is_some())
            .finish_non_exhaustive()
    }
}

impl IpcClient {
    /// Connect to the bus server with Noise IK encrypted transport.
    ///
    /// `shared_pools`: Pass `Some(&pools)` to share rayon thread pools across
    /// multiple clients in the same process (avoids thread oversubscription).
    /// Pass `None` to create dedicated pools for this connection.
    pub async fn connect(
        sender_id: Uuid,
        path: &Path,
        server_public_key: &[u8; 32],
        client_keypair: &snow::Keypair,
        config: &IpcConfig,
        shared_pools: Option<&SharedPools>,
    ) -> IpcResult<Self> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| IpcError::SocketBind {
                path: path.display().to_string(),
                source: e,
            })?;

        #[cfg(unix)]
        apply_socket_options(&stream, config);

        let server_creds = extract_ucred(&stream)?;
        let local_creds = PeerCredentials::local();

        let (reader, writer) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        let mut writer = tokio::io::BufWriter::new(writer);

        let mut handshake = noise::client_handshake(
            &mut reader, &mut writer,
            server_public_key, client_keypair,
            &local_creds, &server_creds,
            config.handshake_timeout(),
            config.max_frame_size,
        ).await?;

        // Derive directional bulk ciphers. Client is the initiator:
        //   send_cipher = initiator_send (encrypt client→server)
        //   recv_cipher = responder_send (decrypt server→client)
        let bulk_keys = handshake.take_handshake_hash()
            .map(|h| bulk::kdf::derive_bulk_key_pair(&h));
        let (bulk_send_cipher, bulk_recv_cipher) = match bulk_keys {
            Some(kp) => (Some(Arc::new(kp.initiator_send)), Some(Arc::new(kp.responder_send))),
            None => (None, None),
        };

        let mut noise_reader = handshake.reader;
        let mut noise_writer = handshake.writer;

        let (app_tx, mut app_rx) = mpsc::channel::<(u64, Vec<u8>, oneshot::Sender<()>)>(256);
        let (transport_tx, mut transport_rx) = mpsc::channel::<Vec<u8>>(64);
        let (inbound_tx, inbound_rx) = mpsc::channel::<bytes::Bytes>(1024);
        let (app_frame_tx, mut app_frame_rx) = mpsc::channel::<bytes::Bytes>(4096);
        let (bulk_out_tx, mut bulk_out_rx) = mpsc::channel::<Vec<u8>>(64);

        let pending_acks: Arc<parking_lot::Mutex<HashMap<u64, AckWaiter>>> =
            Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let pending_bulk: Arc<parking_lot::Mutex<HashMap<u8, BulkWaiter>>> =
            Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let phase = Arc::new(parking_lot::Mutex::new(ConnectionPhase::Ready));

        let pending_acks_ctrl = Arc::clone(&pending_acks);
        let pending_bulk_ctrl = Arc::clone(&pending_bulk);
        let phase_ctrl = Arc::clone(&phase);
        let transport_tx_ctrl = transport_tx.clone();

        // ---- Bulk receive pipeline (server → client) ----
        // Single delivery channel: moves ZeroizingBuf to the application.
        // Zero copies in the hot path. recv_bulk() is a wrapper over recv_bulk_chunk().
        let (bulk_chunk_tx, bulk_chunk_rx) = mpsc::channel::<BulkChunk>(64);
        let (recv_reassembly_tx, mut recv_reassembly_rx) = mpsc::channel::<bulk::DecryptedChunk>(
            bulk::dispatcher::DEFAULT_REASSEMBLY_CAPACITY,
        );
        let recv_decrypt_pool = match shared_pools {
            Some(sp) => Arc::clone(&sp.decrypt_pool),
            None => bulk::encrypt::build_encrypt_pool(config.encrypt_workers),
        };
        let recv_counters = bulk::BulkCounters::new();
        let mut recv_dispatcher = bulk_recv_cipher.as_ref().map(|cipher| {
            bulk::BulkDispatcher::new(
                Arc::clone(cipher),
                Arc::clone(&recv_decrypt_pool),
                recv_reassembly_tx,
                bulk::DigestAlgorithm::Blake3,
                Arc::clone(&recv_counters),
            )
        });
        let (cancel_recv_tx, mut cancel_recv_rx) = mpsc::channel::<u8>(16);
        let cancelled_recv_streams: Arc<parking_lot::Mutex<std::collections::HashSet<u8>>> =
            Arc::new(parking_lot::Mutex::new(std::collections::HashSet::new()));

        // ---- Read task ----
        // Bulk frames are dispatched to rayon DIRECTLY from the read task,
        // bypassing the control loop entirely. This eliminates one channel hop
        // on the recv hot path, closing the send/recv throughput asymmetry.
        // Only control frames go to the control loop via read_frame_tx.
        let (read_frame_tx, mut read_frame_rx) = mpsc::channel::<ReadFrame>(256);
        let read_handle = tokio::spawn(async move {
            let mut reader = reader;
            loop {
                let lane = match lane::read_lane_byte(&mut reader).await {
                    Ok(l) => l,
                    Err(_) => { let _ = read_frame_tx.send(ReadFrame::Disconnected).await; return; }
                };

                if lane == LANE_CONTROL {
                    match noise_reader.read_encrypted_frame(&mut reader).await {
                        Ok(payload) => {
                            if read_frame_tx.send(ReadFrame::Control(payload)).await.is_err() { return; }
                        }
                        Err(_) => { let _ = read_frame_tx.send(ReadFrame::Disconnected).await; return; }
                    }
                } else if lane::is_bulk_lane(lane) {
                    use tokio::io::AsyncReadExt;
                    let mut len_buf = [0u8; 4];
                    if reader.read_exact(&mut len_buf).await.is_err() {
                        let _ = read_frame_tx.send(ReadFrame::Disconnected).await;
                        return;
                    }
                    let len = u32::from_le_bytes(len_buf) as usize;
                    if len > 256 * 1024 {
                        let _ = read_frame_tx.send(ReadFrame::Disconnected).await;
                        return;
                    }
                    let mut body = vec![0u8; len];
                    if reader.read_exact(&mut body).await.is_err() {
                        let _ = read_frame_tx.send(ReadFrame::Disconnected).await;
                        return;
                    }
                    // Dispatch bulk frames directly to rayon — no control loop hop.
                    if let Some(ref mut dispatcher) = recv_dispatcher {
                        if let Err(e) = dispatcher.dispatch(body) {
                            tracing::warn!(error = %e, "client bulk dispatch failed");
                        }
                    }
                } else {
                    let _ = read_frame_tx.send(ReadFrame::Disconnected).await;
                    return;
                }
            }
        });

        // ---- Write task ----
        let (write_error_tx, mut write_error_rx) = mpsc::channel::<()>(4);
        let bulk_counters_write = bulk::BulkCounters::new();
        let bulk_counters = Arc::clone(&bulk_counters_write);
        let send_buf_pool = bulk::BufferPool::new(config.pool_slab_count);
        if writer.flush().await.is_err() {
            return Err(IpcError::ConnectionClosed);
        }
        let write_half = writer.into_inner();
        let write_buf_pool = Arc::clone(&send_buf_pool);
        let write_handle = tokio::spawn(async move {
            let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, write_half);
            let mut bulk_batch: Vec<Vec<u8>> = Vec::with_capacity(64);

            loop {
                enum WriteAction {
                    Transport(Vec<u8>),
                    App { seq: u64, tagged: Vec<u8>, ack_tx: oneshot::Sender<()> },
                    Bulk(Vec<Vec<u8>>),
                    Shutdown,
                }

                let action = tokio::select! {
                    biased;

                    msg = transport_rx.recv() => {
                        match msg {
                            Some(payload) => WriteAction::Transport(payload),
                            None => WriteAction::Shutdown,
                        }
                    }

                    msg = app_rx.recv() => {
                        match msg {
                            Some((seq, tagged, ack_tx)) => WriteAction::App { seq, tagged, ack_tx },
                            None => WriteAction::Shutdown,
                        }
                    }

                    n = bulk_out_rx.recv_many(&mut bulk_batch, 64) => {
                        if n == 0 {
                            WriteAction::Shutdown
                        } else {
                            WriteAction::Bulk(std::mem::take(&mut bulk_batch))
                        }
                    }
                };

                let write_ok = match action {
                    WriteAction::Transport(payload) => {
                        noise_write(&mut noise_writer, &mut writer, &payload).await.is_ok()
                    }
                    WriteAction::App { seq, tagged, ack_tx } => {
                        pending_acks.lock().insert(seq, AckWaiter { tx: ack_tx });
                        let ok = noise_write(&mut noise_writer, &mut writer, &tagged).await.is_ok();
                        if !ok {
                            if let Some(waiter) = pending_acks.lock().remove(&seq) {
                                let _ = waiter.tx.send(());
                            }
                        }
                        ok
                    }
                    WriteAction::Bulk(batch) => {
                        let batch_size = batch.len() as u64;
                        let mut total_bytes = 0u64;
                        let mut ok = true;
                        for frame_body in batch.iter() {
                            total_bytes += frame_body.len() as u64;
                            if writer.write_all(&[LANE_BULK]).await.is_err() { ok = false; break; }
                            let len = (frame_body.len() as u32).to_le_bytes();
                            if writer.write_all(&len).await.is_err() { ok = false; break; }
                            if writer.write_all(frame_body).await.is_err() { ok = false; break; }
                        }
                        if ok { ok = writer.flush().await.is_ok(); }
                        if ok {
                            bulk_counters_write.frames_sent.fetch_add(batch_size, std::sync::atomic::Ordering::Relaxed);
                            bulk_counters_write.bytes_sent.fetch_add(total_bytes, std::sync::atomic::Ordering::Relaxed);
                        }
                        for slab in batch {
                            write_buf_pool.replenish(slab);
                        }
                        ok
                    }
                    WriteAction::Shutdown => {
                        let _ = writer.flush().await;
                        break;
                    }
                };

                if !write_ok {
                    let _ = write_error_tx.try_send(());
                    return;
                }
            }
        });

        // ---- Capture config values for the control loop ----
        let heartbeat_interval = config.heartbeat_interval();
        let heartbeat_pong_timeout = config.heartbeat_pong_timeout();
        let heartbeat_max_misses = config.heartbeat_max_misses;
        let drain_timeout = config.drain_timeout();

        // ---- Inbound delivery task (backpressure without blocking control loop) ----
        let inbound_handle = tokio::spawn(async move {
            while let Some(frame) = app_frame_rx.recv().await {
                if inbound_tx.send(frame).await.is_err() {
                    // User dropped inbound_rx — drain remaining frames silently.
                    // Do NOT exit immediately — drain keeps app_frame_tx open so the
                    // control loop's try_send doesn't see Closed prematurely.
                    while app_frame_rx.recv().await.is_some() {}
                    break;
                }
            }
        });

        // ---- Reassembly task (separate from control loop — never blocks dispatch) ----
        let reassembly_cancelled = Arc::clone(&cancelled_recv_streams);
        let reassembly_handle = tokio::spawn(async move {
            let mut reassemblers: std::collections::HashMap<u8, bulk::reassembly::Reassembler> =
                std::collections::HashMap::new();
            let mut batch: Vec<bulk::DecryptedChunk> = Vec::with_capacity(64);

            loop {
                let n = recv_reassembly_rx.recv_many(&mut batch, 64).await;
                if n == 0 { break; }

                for chunk in batch.drain(..) {
                    // Drain cancel signals inline — catches signals arriving mid-batch.
                    // Adds to the shared cancelled_recv_streams set (same Arc that
                    // cancel_recv_bulk() and recv_bulk() use). Also removes the
                    // reassembler so future chunks don't accumulate state.
                    while let Ok(sid) = cancel_recv_rx.try_recv() {
                        reassemblers.remove(&sid);
                        reassembly_cancelled.lock().insert(sid);
                    }

                    let sid = chunk.stream_id;
                    if reassembly_cancelled.lock().contains(&sid) {
                        if chunk.is_last {
                            reassembly_cancelled.lock().remove(&sid);
                        }
                        continue;
                    }
                    let reassembler = reassemblers
                        .entry(sid)
                        .or_insert_with(|| bulk::reassembly::Reassembler::new(1024));
                    match reassembler.process(chunk) {
                        Ok(delivered) => {
                            let mut stream_finished = false;
                            for r in delivered {
                                let is_last = r.is_last;
                                // send().await provides backpressure — never drops chunks.
                                if bulk_chunk_tx.send(BulkChunk {
                                    stream_id: sid,
                                    chunk_seq: r.chunk_seq,
                                    data: r.plaintext,
                                    is_last,
                                    reservation: r.reservation,
                                    per_conn_reservation: r.per_conn_reservation,
                                }).await.is_err() {
                                    return; // receiver dropped — connection shutting down
                                }
                                if is_last { stream_finished = true; }
                            }
                            if stream_finished { reassemblers.remove(&sid); }
                        }
                        Err(e) => {
                            tracing::warn!(stream_id = sid, error = %e, "client bulk reassembly error");
                            reassemblers.remove(&sid);
                        }
                    }
                }
            }
        });

        // ---- Control loop (heartbeat, transport frames, dispatch — NO reassembly) ----
        let io_handle = tokio::spawn(async move {
            let mut heartbeat_timer = tokio::time::interval_at(
                tokio::time::Instant::now() + heartbeat_interval,
                heartbeat_interval,
            );
            heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_activity = Instant::now();
            let mut awaiting_pong = false;
            let mut heartbeat_misses: u32 = 0;
            let mut pong_deadline: Option<tokio::time::Instant> = None;

            loop {
                let pong_sleep = pong_deadline
                    .map(|d| tokio::time::sleep_until(d))
                    .unwrap_or_else(|| tokio::time::sleep(Duration::from_secs(3600)));

                tokio::select! {
                    biased;

                    _ = write_error_rx.recv() => {
                        resolve_all_lost(&pending_acks_ctrl, &pending_bulk_ctrl);
                        break;
                    }

                    _ = pong_sleep, if awaiting_pong => {
                        heartbeat_misses += 1;
                        awaiting_pong = false;
                        pong_deadline = None;
                        if heartbeat_misses >= heartbeat_max_misses {
                            tracing::warn!("client heartbeat dead");
                            *phase_ctrl.lock() = ConnectionPhase::Dead;
                            resolve_all_lost(&pending_acks_ctrl, &pending_bulk_ctrl);
                            break;
                        }
                    }

                    _ = heartbeat_timer.tick() => {
                        if last_activity.elapsed() >= heartbeat_interval && !awaiting_pong {
                            let ping = encode_heartbeat_ping(wall_ms());
                            match transport_tx_ctrl.try_send(ping) {
                                Ok(()) => {
                                    awaiting_pong = true;
                                    pong_deadline = Some(tokio::time::Instant::now() + heartbeat_pong_timeout);
                                }
                                Err(mpsc::error::TrySendError::Full(_)) => {}
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    resolve_all_lost(&pending_acks_ctrl, &pending_bulk_ctrl);
                                    break;
                                }
                            }
                        }
                    }

                    frame = read_frame_rx.recv() => {
                        match frame {
                            Some(ReadFrame::Control(payload)) => {
                                last_activity = Instant::now();
                                if payload.is_empty() { continue; }
                                let tag = payload[0];
                                let body = &payload[1..];

                                if TransportTag::is_transport(tag) {
                                    handle_client_transport_frame(
                                        tag, body,
                                        &pending_acks_ctrl, &pending_bulk_ctrl,
                                        &transport_tx_ctrl,
                                        &mut heartbeat_misses,
                                        &mut awaiting_pong,
                                        &mut pong_deadline,
                                    );
                                } else if TransportTag::is_application(tag) {
                                    match app_frame_tx.try_send(bytes::Bytes::copy_from_slice(body)) {
                                        Ok(()) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            tracing::warn!("inbound channel full — application frame dropped");
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                                    }
                                }
                            }
                            Some(ReadFrame::Bulk(_)) => {
                                last_activity = Instant::now();
                            }
                            Some(ReadFrame::Disconnected) | None => {
                                resolve_all_lost(&pending_acks_ctrl, &pending_bulk_ctrl);
                                break;
                            }
                        }
                    }
                }
            }

            *phase_ctrl.lock() = ConnectionPhase::Closed;
            drop(transport_tx_ctrl);
            drop(app_frame_tx);
            // Abort read task first — drops recv_dispatcher inside it,
            // which closes reassembly_tx. Then await reassembly drain.
            read_handle.abort();
            let _ = read_handle.await;
            let mut reassembly_handle = reassembly_handle;
            match tokio::time::timeout(drain_timeout, &mut reassembly_handle).await {
                Ok(_) => {}
                Err(_) => {
                    reassembly_handle.abort();
                    let _ = reassembly_handle.await;
                }
            }
            let mut write_handle = write_handle;
            match tokio::time::timeout(drain_timeout, &mut write_handle).await {
                Ok(_) => {}
                Err(_) => {
                    write_handle.abort();
                    let _ = write_handle.await;
                }
            }
        });

        let encrypt_pool = match shared_pools {
            Some(sp) => Arc::clone(&sp.encrypt_pool),
            None => bulk::encrypt::build_encrypt_pool(config.encrypt_workers),
        };
        let send_nonce_ctr = Arc::new(bulk::NonceCounter::new());

        Ok(Self {
            sender_id,
            msg_ctx: MessageContext::new(sender_id),
            app_tx,
            transport_tx,
            inbound_rx,
            bulk_chunk_rx,
            buffered_chunks: VecDeque::new(),
            epoch: Instant::now(),
            io_handle,
            inbound_handle,
            bulk_cipher: bulk_send_cipher,
            bulk_out_tx,
            encrypt_pool,
            send_buf_pool,
            send_nonce_ctr,
            next_seq: AtomicU64::new(1),
            pending_bulk,
            phase,
            bulk_counters,
            cancel_recv_tx,
            cancelled_recv_streams,
            max_frame_size: config.max_frame_size,
        })
    }

    /// Connect with automatic retry and backoff.
    pub async fn connect_with_retry(
        sender_id: Uuid,
        path: &Path,
        server_pub: &[u8; 32],
        client_kp: &snow::Keypair,
        config: &IpcConfig,
        max_attempts: u32,
        backoff: Duration,
        shared_pools: Option<&SharedPools>,
    ) -> IpcResult<Self> {
        let mut last_err = None;
        for attempt in 1..=max_attempts {
            match Self::connect(sender_id, path, server_pub, client_kp, config, shared_pools).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "connect failed, retrying");
                    last_err = Some(e);
                    if attempt < max_attempts {
                        tokio::time::sleep(backoff * attempt).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or(IpcError::ConnectionClosed))
    }

    /// Send an application frame. Returns when the peer acks or timeout/error.
    /// Frame size is checked here BEFORE entering the write task. A rejection
    /// here is graceful — it returns WriteFailed without killing the connection.
    /// The NoiseWriter also checks as a defense-in-depth backstop.
    #[allow(clippy::cast_possible_truncation)]
    pub async fn send_frame(&self, payload: &[u8], ack_timeout: Duration) -> SendOutcome {
        if payload.len() > self.max_frame_size as usize {
            return SendOutcome::WriteFailed(IpcError::FrameTooLarge {
                size: payload.len() as u32,
                max: self.max_frame_size,
            });
        }
        if !self.phase.lock().can_send() {
            return SendOutcome::ConnectionNotActive;
        }

        let seq = self.next_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tagged = tag_application_frame(seq, payload);
        let (ack_tx, ack_rx) = oneshot::channel();

        if self.app_tx.send((seq, tagged, ack_tx)).await.is_err() {
            return SendOutcome::WriteFailed(IpcError::OutboundClosed);
        }

        match tokio::time::timeout(ack_timeout, ack_rx).await {
            Ok(Ok(())) => SendOutcome::Delivered,
            Ok(Err(_)) => SendOutcome::WriteFailed(IpcError::ConnectionClosed),
            Err(_) => SendOutcome::AckTimeout,
        }
    }

    /// Send a bulk payload on the default stream (stream_id 0).
    pub async fn send_bulk(&self, payload: &[u8], ack_timeout: Duration) -> BulkOutcome {
        self.send_bulk_on_stream(0, payload, ack_timeout).await
    }

    /// Send a bulk payload on a specific stream_id (0-255).
    pub async fn send_bulk_on_stream(
        &self, stream_id: u8, payload: &[u8], ack_timeout: Duration,
    ) -> BulkOutcome {
        let cipher = match self.bulk_cipher.as_ref() {
            Some(c) => c,
            None => return BulkOutcome::ConnectionLost,
        };

        if !self.phase.lock().can_send() {
            return BulkOutcome::ConnectionLost;
        }

        // Atomic check-and-insert: the lock is held across both the
        // contains_key check AND the insert. Without this, two concurrent
        // callers can both pass contains_key before either inserts (TOCTOU).
        let (outcome_tx, outcome_rx) = oneshot::channel();
        {
            let mut pending = self.pending_bulk.lock();
            if pending.contains_key(&stream_id) {
                return BulkOutcome::StreamBusy;
            }
            pending.insert(stream_id, BulkWaiter {
                tx: outcome_tx,
                started_at: Instant::now(),
            });
        }

        bulk::transfer::send_payload(
            &self.encrypt_pool,
            cipher,
            &self.send_nonce_ctr,
            &self.send_buf_pool,
            self.bulk_out_tx.clone(),
            stream_id,
            payload,
            bulk::DigestAlgorithm::Blake3,
        );

        match tokio::time::timeout(ack_timeout, outcome_rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => BulkOutcome::ConnectionLost,
            Err(_) => {
                self.pending_bulk.lock().remove(&stream_id);
                let frames_written = self.bulk_counters.frames_sent.load(
                    std::sync::atomic::Ordering::Relaxed);
                let bytes_written = self.bulk_counters.bytes_sent.load(
                    std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(
                    frames_written, bytes_written,
                    nonces_issued = self.send_nonce_ctr.current(),
                    "bulk ack timeout"
                );
                BulkOutcome::AckTimeout {
                    bytes_sent: bytes_written,
                    chunks_sent: frames_written,
                }
            }
        }
    }

    /// Cancel an in-flight outbound bulk transfer on a specific stream.
    /// Sends BULK_CANCEL to the server and resolves the pending send with Cancelled.
    pub async fn cancel_bulk(&self, stream_id: u8) {
        let next_nonce = self.send_nonce_ctr.current();
        let cancel = encode_bulk_cancel(stream_id, next_nonce);
        let _ = self.transport_tx.send(cancel).await;
        if let Some(waiter) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = waiter.tx.send(BulkOutcome::Cancelled);
        }
    }

    /// Cancel an in-flight inbound bulk receive on a specific stream.
    /// Drops the client's recv_reassembler and recv_accumulator for that stream.
    /// Subsequent chunks for this stream are silently dropped.
    /// Other streams are unaffected.
    ///
    /// Two-phase cancel:
    /// 1. Adds to cancelled_recv_streams (shared set) — recv_bulk() sees this
    ///    immediately and skips chunks for this stream instead of blocking on them.
    /// 2. Sends to cancel_recv_tx — reassembly task removes the reassembler and
    ///    drops future chunks before they reach bulk_chunk_tx.
    pub async fn cancel_recv_bulk(&self, stream_id: u8) {
        self.cancelled_recv_streams.lock().insert(stream_id);
        let _ = self.cancel_recv_tx.send(stream_id).await;
    }

    /// Receive next inbound application frame. None on disconnect.
    pub async fn recv(&mut self) -> Option<bytes::Bytes> {
        self.inbound_rx.recv().await
    }

    /// Receive next completed bulk payload from the server (buffered).
    ///
    /// Wrapper over `recv_bulk_chunk()` — accumulates chunks into a
    /// contiguous `Vec<u8>`. One copy per chunk (extend_from_slice),
    /// deferred to this call site rather than the control loop hot path.
    ///
    /// Cancellation-aware: if the target stream is cancelled via
    /// `cancel_recv_bulk()`, discards accumulated data and retries on
    /// the next available stream. Does not block on cancelled streams.
    ///
    /// WARNING: Buffers the ENTIRE payload in memory. A 4GB transfer
    /// allocates 4GB of RAM. For transfers >10MB, use `recv_bulk_chunk()`
    /// which delivers O(65KB) chunks and never holds the full payload.
    pub async fn recv_bulk(&mut self) -> Option<(u8, Vec<u8>)> {
        let mut payload = Vec::new();
        let mut target_sid: Option<u8> = None;

        // Purge buffered chunks for cancelled streams before starting.
        self.drain_cancelled_buffered_chunks();

        loop {
            // Check if our target stream was cancelled while we were accumulating.
            if let Some(sid) = target_sid {
                if self.cancelled_recv_streams.lock().contains(&sid) {
                    // Target cancelled — discard accumulated data, reset, retry.
                    payload.clear();
                    target_sid = None;
                    self.drain_cancelled_buffered_chunks();
                    continue;
                }
            }

            // Drain buffered chunks first — FIFO order via VecDeque.
            let chunk = if let Some(idx) = target_sid.and_then(|t| {
                self.buffered_chunks.iter().position(|c| c.stream_id == t)
            }) {
                self.buffered_chunks.remove(idx).unwrap()
            } else if target_sid.is_none() {
                // Find first non-cancelled chunk in buffer or channel.
                match self.next_non_cancelled_chunk().await {
                    Some(c) => c,
                    None => return None,
                }
            } else {
                self.bulk_chunk_rx.recv().await?
            };

            let sid = chunk.stream_id;

            // Skip chunks for cancelled streams.
            if self.cancelled_recv_streams.lock().contains(&sid) {
                continue;
            }

            let first_sid = *target_sid.get_or_insert(sid);

            if sid != first_sid {
                self.buffered_chunks.push_back(chunk);
                continue;
            }

            if chunk.is_last {
                debug_assert!(chunk.data.is_empty(), "is_last chunk must have empty data");
                return Some((sid, payload));
            }
            payload.extend_from_slice(&chunk.data);
        }
    }

    /// Receive the next bulk data chunk from the server (streaming).
    ///
    /// Returns one chunk (~65KB) at a time. Memory usage is O(chunk_size)
    /// regardless of total transfer size. When `chunk.is_last` is true,
    /// `chunk.data` is empty and the transfer is complete (Merkle verified).
    ///
    /// Cancellation-aware: skips chunks for cancelled streams.
    ///
    /// Use this instead of `recv_bulk` for large transfers (>10MB) to
    /// avoid buffering the entire payload in memory.
    pub async fn recv_bulk_chunk(&mut self) -> Option<BulkChunk> {
        // Drain cancelled chunks from buffer.
        self.drain_cancelled_buffered_chunks();

        if let Some(chunk) = self.buffered_chunks.pop_front() {
            return Some(chunk);
        }

        loop {
            let chunk = self.bulk_chunk_rx.recv().await?;
            if self.cancelled_recv_streams.lock().contains(&chunk.stream_id) {
                continue;
            }
            return Some(chunk);
        }
    }

    /// Remove all buffered chunks belonging to cancelled streams.
    fn drain_cancelled_buffered_chunks(&mut self) {
        let cancelled = self.cancelled_recv_streams.lock();
        if cancelled.is_empty() {
            return;
        }
        self.buffered_chunks.retain(|c| !cancelled.contains(&c.stream_id));
    }

    /// Get the next non-cancelled chunk from buffer or channel.
    async fn next_non_cancelled_chunk(&mut self) -> Option<BulkChunk> {
        // Check buffer first.
        {
            let cancelled = self.cancelled_recv_streams.lock();
            if let Some(idx) = self.buffered_chunks.iter().position(|c| !cancelled.contains(&c.stream_id)) {
                return self.buffered_chunks.remove(idx);
            }
        }

        // Buffer empty or all cancelled — read from channel.
        loop {
            let chunk = self.bulk_chunk_rx.recv().await?;
            if self.cancelled_recv_streams.lock().contains(&chunk.stream_id) {
                continue;
            }
            return Some(chunk);
        }
    }

    pub fn bulk_cipher(&self) -> Option<&Arc<bulk::BulkCipher>> {
        self.bulk_cipher.as_ref()
    }

    pub fn bulk_counters(&self) -> &Arc<bulk::BulkCounters> {
        &self.bulk_counters
    }

    pub fn encrypt_nonces_issued(&self) -> u64 {
        self.send_nonce_ctr.current()
    }

    pub fn bulk_channel_queued(&self) -> usize {
        let issued = self.send_nonce_ctr.current();
        let sent = self.bulk_counters.frames_sent.load(std::sync::atomic::Ordering::Relaxed);
        issued.saturating_sub(sent) as usize
    }

    pub fn sender_id(&self) -> Uuid {
        self.sender_id
    }

    pub fn msg_ctx(&self) -> &MessageContext {
        &self.msg_ctx
    }

    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    pub fn phase(&self) -> ConnectionPhase {
        *self.phase.lock()
    }

    pub async fn send_message<T: Serialize>(
        &self, payload: T, level: SecurityLevel, ack_timeout: Duration,
    ) -> SendOutcome {
        let msg = Message::new(&self.msg_ctx, payload, level, self.epoch);
        let encoded = match encode_frame(&msg) {
            Ok(bytes) => bytes,
            Err(e) => return SendOutcome::WriteFailed(e),
        };
        self.send_frame(&encoded, ack_timeout).await
    }

    pub async fn send_message_with_correlation<T: Serialize>(
        &self, payload: T, level: SecurityLevel, correlation_id: Uuid, ack_timeout: Duration,
    ) -> SendOutcome {
        let msg = Message::new(&self.msg_ctx, payload, level, self.epoch)
            .with_correlation(correlation_id);
        let encoded = match encode_frame(&msg) {
            Ok(bytes) => bytes,
            Err(e) => return SendOutcome::WriteFailed(e),
        };
        self.send_frame(&encoded, ack_timeout).await
    }

    pub async fn send_message_to_community<T: Serialize>(
        &self, payload: T, level: SecurityLevel, community: String, ack_timeout: Duration,
    ) -> SendOutcome {
        let msg = Message::new(&self.msg_ctx, payload, level, self.epoch)
            .with_community(community);
        let encoded = match encode_frame(&msg) {
            Ok(bytes) => bytes,
            Err(e) => return SendOutcome::WriteFailed(e),
        };
        self.send_frame(&encoded, ack_timeout).await
    }

    /// Gracefully shut down. Timeout of 5s prevents hanging if the server is unresponsive.
    pub async fn shutdown(self) {
        let _ = self.transport_tx.send(encode_shutdown()).await;
        drop(self.bulk_out_tx);
        drop(self.app_tx);
        drop(self.transport_tx);
        let mut handle = self.io_handle;
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(_) => {}
            Err(_) => {
                handle.abort();
                let _ = handle.await;
            }
        }
        // Inbound task: signal drain via app_frame_tx drop (already happened inside
        // io_handle's exit path). Await with timeout — if user isn't reading, abort.
        let mut inbound = self.inbound_handle;
        match tokio::time::timeout(Duration::from_secs(1), &mut inbound).await {
            Ok(_) => {}
            Err(_) => {
                inbound.abort();
                let _ = inbound.await;
            }
        }
    }
}

impl IpcClient {
    pub fn into_split(mut self) -> (std::sync::Arc<Self>, mpsc::Receiver<bytes::Bytes>) {
        let (_, dummy_rx) = mpsc::channel(1);
        let real_rx = std::mem::replace(&mut self.inbound_rx, dummy_rx);
        (std::sync::Arc::new(self), real_rx)
    }
}

/// Write a Noise-encrypted frame on lane 0x00 and flush.
async fn noise_write(
    noise_writer: &mut noise::NoiseWriter,
    writer: &mut (impl AsyncWriteExt + Unpin),
    payload: &[u8],
) -> Result<(), ()> {
    if writer.write_all(&[LANE_CONTROL]).await.is_err() { return Err(()); }
    if noise_writer.write_encrypted_frame(writer, payload).await.is_err() { return Err(()); }
    if writer.flush().await.is_err() { return Err(()); }
    Ok(())
}

fn handle_client_transport_frame(
    tag: u8,
    body: &[u8],
    pending_acks: &parking_lot::Mutex<HashMap<u64, AckWaiter>>,
    pending_bulk: &parking_lot::Mutex<HashMap<u8, BulkWaiter>>,
    transport_tx: &mpsc::Sender<Vec<u8>>,
    heartbeat_misses: &mut u32,
    awaiting_pong: &mut bool,
    pong_deadline: &mut Option<tokio::time::Instant>,
) {
    match tag {
        TransportTag::ACK => {
            if body.len() >= 8 {
                let seq = u64::from_le_bytes(body[..8].try_into().unwrap());
                if let Some(waiter) = pending_acks.lock().remove(&seq) {
                    let _ = waiter.tx.send(());
                }
            }
        }
        TransportTag::HEARTBEAT_PING => {
            if body.len() >= 8 {
                let ts = u64::from_le_bytes(body[..8].try_into().unwrap());
                let _ = transport_tx.try_send(encode_heartbeat_pong(ts));
            }
        }
        TransportTag::HEARTBEAT_PONG => {
            *heartbeat_misses = 0;
            *awaiting_pong = false;
            *pong_deadline = None;
        }
        TransportTag::BULK_ACK => {
            if body.len() >= 17 {
                let stream_id = body[0];
                let chunks = u64::from_le_bytes(body[1..9].try_into().unwrap());
                let bytes = u64::from_le_bytes(body[9..17].try_into().unwrap());
                if let Some(waiter) = pending_bulk.lock().remove(&stream_id) {
                    let _ = waiter.tx.send(BulkOutcome::Delivered {
                        bytes_transferred: bytes,
                        duration: waiter.started_at.elapsed(),
                        chunks,
                    });
                }
            }
        }
        TransportTag::BULK_NACK => {
            if body.len() >= 2 {
                let stream_id = body[0];
                let reason: BulkNackReason = postcard::from_bytes(&body[1..])
                    .unwrap_or(BulkNackReason::ReassemblyOverflow);
                let outcome = match reason {
                    BulkNackReason::DigestMismatch => BulkOutcome::IntegrityFailed,
                    BulkNackReason::Cancelled => BulkOutcome::Cancelled,
                    _ => BulkOutcome::IntegrityFailed,
                };
                if let Some(waiter) = pending_bulk.lock().remove(&stream_id) {
                    let _ = waiter.tx.send(outcome);
                }
            }
        }
        TransportTag::BULK_CANCEL => {}
        TransportTag::SHUTDOWN => {
            let _ = transport_tx.try_send(encode_shutdown_ack());
        }
        TransportTag::SHUTDOWN_ACK => {}
        _ => {}
    }
}

fn resolve_all_lost(
    pending_acks: &parking_lot::Mutex<HashMap<u64, AckWaiter>>,
    pending_bulk: &parking_lot::Mutex<HashMap<u8, BulkWaiter>>,
) {
    for (_, waiter) in pending_acks.lock().drain() {
        let _ = waiter.tx.send(());
    }
    for (_, waiter) in pending_bulk.lock().drain() {
        let _ = waiter.tx.send(BulkOutcome::ConnectionLost);
    }
}

#[cfg(unix)]
fn apply_socket_options(stream: &UnixStream, config: &IpcConfig) {
    #![allow(unsafe_code)]
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    if let Some(sndbuf) = config.uds_sndbuf {
        unsafe {
            let val = sndbuf as libc::c_int;
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                (&raw const val).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
    if let Some(rcvbuf) = config.uds_rcvbuf {
        unsafe {
            let val = rcvbuf as libc::c_int;
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&raw const val).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
