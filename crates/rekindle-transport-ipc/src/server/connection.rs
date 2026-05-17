//! Per-connection lifecycle: handshake → read task → control loop → teardown.
//!
//! Architecture: three tasks per connection.
//!
//! 1. READ TASK (tokio::spawn): owns BufReader<OwnedReadHalf> + NoiseReader.
//!    Reads complete frames in a simple loop with NO select!. No cancellation
//!    issue because there is no competing branch. Sends ReadFrame to the
//!    control loop via mpsc. Exits on EOF or error (sends ReadFrame::Disconnected).
//!
//! 2. WRITE TASK (tokio::spawn): owns BufWriter<OwnedWriteHalf> + NoiseWriter.
//!    Receives frames from 4 channels (transport, response, event, bulk) via
//!    biased select!. All write_all calls happen OUTSIDE select! (cancellation-safe).
//!
//! 3. CONTROL LOOP (this function, after handshake): NO async I/O. Only channel
//!    recvs and timers in select!. All cancellation-safe. Handles ack resolution,
//!    heartbeat, reassembly, bulk delivery, cancel, state transitions.
//!
//! Bridge-free: rayon workers call blocking_send directly into tokio mpsc.
//! pool.spawn() exclusively, never pool.scope().

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::backpressure::GlobalMemoryGuard;
use crate::bulk::{self, BulkCounters, BulkDispatcher, BufferPool};
use crate::bulk::dispatcher::DecryptedChunk;
use crate::bulk::reassembly::Reassembler;
use crate::config::IpcConfig;
use crate::envelope::{SecurityLevel, SharedFrame};
use crate::frame::codec::MAX_FRAME_SIZE;
use crate::frame::lane;
use crate::noise;
use crate::socket::PeerCredentials;
use crate::transport_frame::*;

use super::state::{ConnectionState, ServerState, TokenBucket};
use super::write_loop;
use super::FrameRouter;

pub struct ConnResources<R: FrameRouter> {
    pub state: Arc<ServerState>,
    pub conn_id: u64,
    pub stream: tokio::net::UnixStream,
    pub resp_tx: mpsc::Sender<Bytes>,
    pub event_tx: mpsc::Sender<SharedFrame>,
    pub resp_rx: mpsc::Receiver<Bytes>,
    pub event_rx: mpsc::Receiver<SharedFrame>,
    pub peer: PeerCredentials,
    pub keypair: Arc<snow::Keypair>,
    pub router: Arc<R>,
    pub config: Arc<IpcConfig>,
    pub encrypt_pool: Arc<rayon::ThreadPool>,
    pub decrypt_pool: Arc<rayon::ThreadPool>,
    pub buffer_pool: Arc<BufferPool>,
    pub bulk_counters: Arc<BulkCounters>,
    pub cancel_token: CancellationToken,
    pub memory_guard: Arc<GlobalMemoryGuard>,
}

pub async fn handle_connection<R: FrameRouter>(res: ConnResources<R>) {
    let ConnResources {
        state, conn_id, stream,
        resp_tx, event_tx, resp_rx, event_rx,
        peer, keypair, router, config,
        encrypt_pool, decrypt_pool, buffer_pool, bulk_counters,
        cancel_token, memory_guard,
    } = res;

    #[cfg(unix)]
    apply_socket_options(&stream, &config);

    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::with_capacity(8192, reader);
    let mut handshake_writer = tokio::io::BufWriter::with_capacity(8192, writer);
    let local_creds = PeerCredentials::local();
    let connected_at = Instant::now();

    // ---- Channels ----
    let (transport_tx, transport_rx) = mpsc::channel::<Vec<u8>>(64);
    let (write_error_tx, mut write_error_rx) = mpsc::channel::<write_loop::WriteError>(4);
    let (cancel_tx, mut cancel_rx) = mpsc::channel::<u8>(16);
    let (read_frame_tx, mut read_frame_rx) = mpsc::channel::<ReadFrame>(256);

    // ---- Noise IK handshake ----
    let mut hs = match noise::server_handshake(
        &mut reader, &mut handshake_writer,
        &keypair, &local_creds, &peer,
        config.handshake_timeout(),
        config.max_frame_size,
    ).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(conn_id, error = %e, "handshake failed");
            router.on_connection_state_changed(&state, conn_id, ConnectionPhase::Handshaking, ConnectionPhase::Closed);
            return;
        }
    };

    // ---- Derive directional bulk ciphers ----
    // Two keys: initiator_send (client→server) and responder_send (server→client).
    // Using a single symmetric key would cause AES-GCM nonce reuse.
    let bulk_keys = hs.take_handshake_hash()
        .map(|h| bulk::kdf::derive_bulk_key_pair(&h));
    // Server is the responder:
    //   recv_cipher = initiator_send (decrypt client→server traffic)
    //   send_cipher = responder_send (encrypt server→client traffic)
    let (bulk_recv_cipher, bulk_send_cipher) = match bulk_keys {
        Some(kp) => (Some(Arc::new(kp.initiator_send)), Some(Arc::new(kp.responder_send))),
        None => (None, None),
    };
    let bulk_session_nonce = Arc::new(bulk::NonceCounter::new());

    // ---- Bulk plane channels (rayon → control loop, rayon → write loop) ----
    let (bulk_out_tx, bulk_out_rx) = mpsc::channel::<Vec<u8>>(64);
    // Bounded reassembly channel: rayon workers call blocking_send.
    // Capacity bounds memory to O(capacity × chunk_size) regardless of
    // total transfer size. Workers park when full — correct backpressure.
    let (reassembly_tx, mut reassembly_rx) = mpsc::channel::<DecryptedChunk>(
        bulk::dispatcher::DEFAULT_REASSEMBLY_CAPACITY,
    );

    let digest_algo = bulk::DigestAlgorithm::default();
    let per_conn_guard = if config.per_connection_memory_limit > 0 {
        Some(Arc::new(GlobalMemoryGuard::new(config.per_connection_memory_limit)))
    } else {
        None
    };
    let bulk_dispatcher = bulk_recv_cipher.as_ref().map(|cipher| {
        let mut d = BulkDispatcher::new(
            Arc::clone(cipher), Arc::clone(&decrypt_pool),
            reassembly_tx, digest_algo,
            Arc::clone(&bulk_counters),
        );
        d = d.with_memory_guard(Arc::clone(&memory_guard));
        if let Some(ref pcg) = per_conn_guard {
            d = d.with_per_connection_guard(Arc::clone(pcg));
        }
        d
    });
    // Reassembly task gets its own clone of transport_tx for sending BulkAck/Nack.
    let reassembly_transport_tx = transport_tx.clone();

    // ---- Register connection state ----
    state.connections.insert(conn_id, ConnectionState {
        agent_id: std::sync::OnceLock::new(),
        verified_name: None,
        response_tx: resp_tx,
        event_tx,
        peer,
        security_clearance: SecurityLevel::Open,
        connected_at,
        rate_limiter: TokenBucket::new(config.rate_limit_per_peer_per_sec, 1000),
        bulk_nonce_counter: if bulk_recv_cipher.is_some() { Some(Arc::clone(&bulk_session_nonce)) } else { None },
        cancel_bulk_tx: cancel_tx,
        phase: parking_lot::Mutex::new(ConnectionPhase::Ready),
        pending_acks: parking_lot::Mutex::new(std::collections::HashMap::new()),
        pending_bulk: parking_lot::Mutex::new(std::collections::HashMap::new()),
        transport_tx: transport_tx.clone(),
        bulk_out_tx: bulk_out_tx.clone(),
        encrypt_pool,
        bulk_cipher: bulk_send_cipher.clone(),
        bulk_send_nonce: Arc::new(bulk::NonceCounter::new()),
        bulk_send_pool: Arc::clone(&buffer_pool),
    });
    router.on_connection_state_changed(&state, conn_id, ConnectionPhase::Handshaking, ConnectionPhase::Ready);

    // ---- Spawn write task ----
    let write_half = handshake_writer.into_inner();
    let write_handle = tokio::spawn(write_loop::run(
        hs.writer, write_half,
        transport_rx, resp_rx, event_rx, bulk_out_rx,
        Arc::clone(&buffer_pool), Arc::clone(&bulk_counters),
        write_error_tx, conn_id,
    ));

    // ---- Spawn read task ----
    // Bulk frames are dispatched to rayon directly from the read task,
    // bypassing the control loop. Only control frames go to the control loop.
    let noise_reader = hs.reader;
    let read_bulk_counters = Arc::clone(&bulk_counters);
    let read_nack_tx = transport_tx.clone();
    let read_handle = tokio::spawn(read_task(
        reader, noise_reader, read_frame_tx, bulk_dispatcher, read_bulk_counters,
        read_nack_tx,
    ));

    // ---- Reassembly task (separate — never blocks dispatch or heartbeat) ----
    let reassembly_router = Arc::clone(&router);
    let reassembly_state = Arc::clone(&state);
    let reassembly_counters = Arc::clone(&bulk_counters);
    let reassembly_handle = tokio::spawn(async move {
        let mut reassemblers: std::collections::HashMap<u8, Reassembler> = std::collections::HashMap::new();
        let mut stream_counters: std::collections::HashMap<u8, (u64, u64)> = std::collections::HashMap::new();
        let mut batch: Vec<DecryptedChunk> = Vec::with_capacity(64);

        loop {
            let n = reassembly_rx.recv_many(&mut batch, 64).await;
            if n == 0 { break; }

            for chunk in batch.drain(..) {
                // Drain cancel signals inline — catches signals arriving mid-batch.
                while let Ok(sid) = cancel_rx.try_recv() {
                    reassemblers.remove(&sid);
                    stream_counters.remove(&sid);
                }

                let sid = chunk.stream_id;
                let reassembler = reassemblers
                    .entry(sid)
                    .or_insert_with(|| Reassembler::with_algorithm(1024, digest_algo));

                match reassembler.process(chunk) {
                    Ok(delivered) => {
                        let data_count = delivered.iter().filter(|r| !r.is_last).count() as u64;
                        if data_count > 0 {
                            reassembly_counters.chunks_reassembled.fetch_add(
                                data_count, Ordering::Relaxed);
                        }
                        let mut stream_finished = false;
                        for r in &delivered {
                            if !r.is_last {
                                let data_len = r.plaintext.len() as u64;
                                reassembly_router.on_bulk_chunk(
                                    &reassembly_state, conn_id, sid, r.chunk_seq,
                                    &r.plaintext,
                                );
                                let counters = stream_counters.entry(sid).or_insert((0, 0));
                                counters.0 += data_len;
                                counters.1 += 1;
                            } else {
                                stream_finished = true;
                                let (total_bytes, total_chunks) = stream_counters
                                    .remove(&sid)
                                    .unwrap_or((0, 0));
                                reassembly_counters.transfers_completed.fetch_add(1, Ordering::Relaxed);
                                reassembly_router.on_bulk_complete(
                                    &reassembly_state, conn_id, sid,
                                    total_bytes, total_chunks,
                                );
                                // send().await — backpressure, never drops the BulkAck.
                                if reassembly_transport_tx.send(encode_bulk_ack(
                                    sid, total_chunks, total_bytes,
                                )).await.is_err() {
                                    return; // write task dead — exit reassembly
                                }
                            }
                        }
                        if stream_finished {
                            reassemblers.remove(&sid);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(conn_id, stream_id = sid, error = %e, "reassembly error");
                        reassemblers.remove(&sid);
                        // send().await for NACK — must reach the peer.
                        if reassembly_transport_tx.send(
                            encode_bulk_nack(sid, &BulkNackReason::ReassemblyOverflow)
                        ).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    // ---- Heartbeat and idle timeout state (from config) ----
    let heartbeat_interval = config.heartbeat_interval();
    let heartbeat_pong_timeout = config.heartbeat_pong_timeout();
    let heartbeat_max_misses = config.heartbeat_max_misses;
    let idle_timeout = config.idle_timeout();
    let idle_timeout_enabled = config.idle_timeout_ms > 0;

    let mut heartbeat_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut heartbeat_miss_count: u32 = 0;
    let mut last_activity = Instant::now();
    let mut awaiting_pong = false;
    let mut pong_deadline: Option<tokio::time::Instant> = None;

    // ---- Control loop: NO async I/O, only channel recvs and timers ----
    loop {
        let pong_sleep = pong_deadline
            .map(|d| tokio::time::sleep_until(d))
            .unwrap_or_else(|| tokio::time::sleep(Duration::from_secs(3600)));

        let action = tokio::select! {
            biased;

            // P0: server shutdown signal — highest priority.
            _ = cancel_token.cancelled() => ControlAction::Shutdown,

            // P1: write task error.
            err = write_error_rx.recv() => {
                if err.is_some() { ControlAction::WriteFailed } else { ControlAction::Shutdown }
            }

            // P2: pong timeout.
            _ = pong_sleep, if awaiting_pong => ControlAction::PongTimeout,

            // P3: heartbeat tick.
            _ = heartbeat_timer.tick() => ControlAction::HeartbeatTick,

            // P4: inbound frames from socket (read task).
            frame = read_frame_rx.recv() => {
                match frame {
                    Some(ReadFrame::Control(payload)) => ControlAction::RecvControl(payload),
                    Some(ReadFrame::Bulk(_)) => continue, // bulk dispatched in read task
                    Some(ReadFrame::Disconnected) | None => ControlAction::Shutdown,
                }
            }
        };

        // ---- Execute action ----
        match action {
            ControlAction::WriteFailed => {
                tracing::warn!(conn_id, "write loop error");
                transition_and_notify(&state, &router, conn_id, ConnectionPhase::Dead);
                break;
            }
            ControlAction::PongTimeout => {
                heartbeat_miss_count += 1;
                awaiting_pong = false;
                pong_deadline = None;
                if heartbeat_miss_count >= heartbeat_max_misses {
                    tracing::warn!(conn_id, misses = heartbeat_miss_count, "heartbeat dead");
                    transition_and_notify(&state, &router, conn_id, ConnectionPhase::Dead);
                    break;
                } else {
                    tracing::debug!(conn_id, misses = heartbeat_miss_count, "heartbeat degraded");
                    transition_and_notify(&state, &router, conn_id, ConnectionPhase::Degraded);
                }
            }
            ControlAction::HeartbeatTick => {
                // Check idle timeout: no activity for idle_timeout_ms.
                if idle_timeout_enabled && last_activity.elapsed() >= idle_timeout {
                    tracing::info!(conn_id, elapsed_ms = last_activity.elapsed().as_millis() as u64, "idle timeout");
                    transition_and_notify(&state, &router, conn_id, ConnectionPhase::Dead);
                    break;
                }
                // Send heartbeat ping if no recent activity.
                if last_activity.elapsed() >= heartbeat_interval && !awaiting_pong {
                    let ping = encode_heartbeat_ping(wall_ms());
                    match transport_tx.try_send(ping) {
                        Ok(()) => {
                            awaiting_pong = true;
                            pong_deadline = Some(tokio::time::Instant::now() + heartbeat_pong_timeout);
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            // Write task backpressured — skip this tick.
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            transition_and_notify(&state, &router, conn_id, ConnectionPhase::Dead);
                            break;
                        }
                    }
                }
            }
            ControlAction::RecvControl(payload) => {
                last_activity = Instant::now();
                if !payload.is_empty() {
                    let tag = payload[0];

                    if TransportTag::is_transport(tag) {
                        let body = &payload[1..];
                        handle_transport_frame(
                            tag, body, &state, conn_id, &transport_tx,
                            &mut heartbeat_miss_count, &mut awaiting_pong, &mut pong_deadline,
                        );
                    } else if TransportTag::is_application(tag) {
                        // Rate limit: check global BEFORE per-peer.
                        // Don't consume a per-peer token if global rejects.
                        let global_ok = state.global_rate_limiter.try_consume();
                        let rate_ok = if global_ok {
                            state.connections.get(&conn_id)
                                .map(|conn| conn.rate_limiter.try_consume())
                                .unwrap_or(false)
                        } else {
                            false
                        };

                        if !rate_ok {
                            tracing::warn!(conn_id, global = global_ok, "rate limit exceeded, dropping frame");
                            // Still send ACK so the client doesn't retry — the frame
                            // was received, just not routed.
                            if let Some((seq, _)) = parse_application_frame(&payload) {
                                let _ = transport_tx.try_send(encode_ack(seq));
                            }
                        } else if let Some((seq, app_payload)) = parse_application_frame(&payload) {
                            // ACK the frame regardless of size — the client already
                            // sent it, acking prevents retransmit storms.
                            let _ = transport_tx.try_send(encode_ack(seq));

                            // Application-layer max_frame_size check on the decrypted
                            // payload. Rejection is graceful — connection survives.
                            if app_payload.len() > config.max_frame_size as usize {
                                tracing::warn!(conn_id, payload_len = app_payload.len(),
                                    max = config.max_frame_size, "oversized app frame, dropping");
                                // Frame is ACK'd but not routed. Connection survives.
                                continue;
                            }

                            if let Some(conn) = state.connections.get(&conn_id) {
                                if conn.current_phase() == ConnectionPhase::Ready {
                                    drop(conn);
                                    transition_and_notify(&state, &router, conn_id, ConnectionPhase::Active);
                                }
                            }
                            router.route_frame(&state, conn_id, Bytes::copy_from_slice(app_payload));
                        }
                    }
                }
            }
            ControlAction::Shutdown => {
                transition_and_notify(&state, &router, conn_id, ConnectionPhase::Dead);
                break;
            }
        }
    }

    // ---- Teardown ----
    // Ordering: drop all channel senders in dependency order so tasks exit without timeout.
    // Critical invariant: every clone of transport_tx must be dropped BEFORE awaiting write_handle.

    if let Some(conn) = state.connections.get(&conn_id) {
        conn.resolve_all_pending_lost();
    }

    if transport_tx.try_send(encode_shutdown()).is_err() {
        tracing::debug!(conn_id, "SHUTDOWN frame dropped (channel full)");
    }

    // 1. Abort read task — drops the dispatcher inside it, which closes
    //    the original reassembly_tx sender. In-flight rayon workers still
    //    hold clones; they complete and send.
    read_handle.abort();
    let _ = read_handle.await;

    // 2. Await reassembly task with abort-on-timeout.
    //    Normal case: rayon workers finish, their tx clones drop, recv_many returns 0, task exits.
    //    Stuck case: rayon workers parked on blocking_send — abort guarantees cleanup.
    //    Either way: reassembly_transport_tx is dropped after this block.
    let mut reassembly_handle = reassembly_handle;
    match tokio::time::timeout(config.drain_timeout(), &mut reassembly_handle).await {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(conn_id, "reassembly drain timeout — aborting");
            reassembly_handle.abort();
            let _ = reassembly_handle.await;
        }
    }

    // 3. Fire Closed notification while DashMap entry still exists for phase lookup.
    transition_and_notify(&state, &router, conn_id, ConnectionPhase::Closed);

    // 5. Remove ConnectionState — extract metadata, then drop immediately.
    //    ConnectionState holds clones of transport_tx and bulk_out_tx that must
    //    be released before the write task can exit.
    let removed_conn = state.connections.remove(&conn_id);
    state.pending_requests.lock().remove_by_conn_id(conn_id);
    #[allow(clippy::cast_possible_truncation)]
    let session_ms = removed_conn.as_ref()
        .map(|(_, c)| c.connected_at.elapsed().as_millis() as u64)
        .unwrap_or(0);
    let conn_name = removed_conn.as_ref()
        .and_then(|(_, c)| c.verified_name.clone());
    drop(removed_conn);

    // 6. Drop remaining senders owned by this function.
    drop(bulk_out_tx);
    drop(transport_tx);

    // 7. Await write task — all senders now dropped, exits immediately.
    //    Abort-on-timeout as safety net (should never fire in normal operation).
    let mut write_handle = write_handle;
    match tokio::time::timeout(config.drain_timeout(), &mut write_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(conn_id, error = %e, "write task panicked");
        }
        Err(_) => {
            tracing::warn!(conn_id, "write task drain timeout — aborting");
            write_handle.abort();
            let _ = write_handle.await;
        }
    }

    tracing::info!(conn_id, session_ms, "connection cleanup");
    if let Some(name) = conn_name {
        state.name_to_conn.write().remove(name.as_ref());
    }
}

enum ControlAction {
    WriteFailed,
    PongTimeout,
    HeartbeatTick,
    RecvControl(bytes::Bytes),
    Shutdown,
}

/// Dedicated read task. Owns the reader exclusively. Simple loop, no select!.
/// Bulk frames are dispatched to rayon directly — no control loop hop.
///
/// When bulk dispatch fails with backpressure, a BULK_NACK is sent for
/// the affected stream so the client's send_bulk resolves instead of
/// hanging forever. Only one NACK per stream per transfer — tracked via
/// `nacked_streams` which is cleared when the fin frame arrives.
async fn read_task(
    mut reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    mut noise_reader: noise::NoiseReader,
    tx: mpsc::Sender<ReadFrame>,
    mut bulk_dispatcher: Option<BulkDispatcher>,
    bulk_counters: Arc<BulkCounters>,
    nack_tx: mpsc::Sender<Vec<u8>>,
) {
    // Tracks which streams have already been NACKed for the current transfer.
    // Prevents sending duplicate NACKs for the same stream. Cleared when
    // the fin frame (kind=0x02) arrives, so the next transfer on the same
    // stream_id starts fresh.
    let mut nacked_streams: std::collections::HashSet<u8> = std::collections::HashSet::new();

    loop {
        let lane = match lane::read_lane_byte(&mut reader).await {
            Ok(l) => l,
            Err(_) => { let _ = tx.send(ReadFrame::Disconnected).await; return; }
        };

        if lane == lane::LANE_CONTROL {
            match noise_reader.read_encrypted_frame(&mut reader).await {
                Ok(payload) => {
                    if tx.send(ReadFrame::Control(payload)).await.is_err() { return; }
                }
                Err(_) => { let _ = tx.send(ReadFrame::Disconnected).await; return; }
            }
        } else if lane::is_bulk_lane(lane) {
            match read_bulk_frame(&mut reader).await {
                Ok(body) => {
                    if let Some(ref mut dispatcher) = bulk_dispatcher {
                        let stream_id = body.first().copied().unwrap_or(0);
                        let is_fin = body.len() > 1 && body[1] == 0x02;

                        // Clear nacked state when fin arrives — the transfer is
                        // ending, so the next transfer on this stream_id starts fresh.
                        if is_fin {
                            nacked_streams.remove(&stream_id);
                        }

                        let len = body.len() as u64;
                        match dispatcher.dispatch(body) {
                            Ok(()) => {
                                bulk_counters.frames_received.fetch_add(1, Ordering::Relaxed);
                                bulk_counters.bytes_received.fetch_add(len, Ordering::Relaxed);
                            }
                            Err(e) => {
                                // Send NACK on first backpressure failure per stream.
                                // Subsequent failures for the same stream are suppressed
                                // to avoid NACK storms. The set is cleared on fin.
                                if matches!(e, crate::bulk::dispatcher::DispatchError::Backpressure { .. }) {
                                    if nacked_streams.insert(stream_id) {
                                        tracing::warn!(stream_id, error = %e, "server bulk dispatch failed, sending NACK");
                                        let _ = nack_tx.try_send(
                                            encode_bulk_nack(stream_id, &BulkNackReason::ReassemblyOverflow)
                                        );
                                    }
                                } else {
                                    tracing::warn!(stream_id, error = %e, "server bulk dispatch failed");
                                }
                            }
                        }
                    }
                }
                Err(_) => { let _ = tx.send(ReadFrame::Disconnected).await; return; }
            }
        } else {
            tracing::warn!(lane, "read task: unknown lane byte, disconnecting");
            let _ = tx.send(ReadFrame::Disconnected).await;
            return;
        }
    }
}

fn handle_transport_frame(
    tag: u8, body: &[u8],
    state: &Arc<ServerState>, conn_id: u64,
    transport_tx: &mpsc::Sender<Vec<u8>>,
    heartbeat_miss_count: &mut u32,
    awaiting_pong: &mut bool,
    pong_deadline: &mut Option<tokio::time::Instant>,
) {
    match tag {
        TransportTag::ACK => {
            if body.len() >= 8 {
                let seq = u64::from_le_bytes(body[..8].try_into().unwrap());
                if let Some(conn) = state.connections.get(&conn_id) {
                    conn.resolve_ack(seq);
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
            *heartbeat_miss_count = 0;
            *awaiting_pong = false;
            *pong_deadline = None;
            if let Some(conn) = state.connections.get(&conn_id) {
                if conn.current_phase() == ConnectionPhase::Degraded {
                    let _ = conn.transition(ConnectionPhase::Active);
                }
            }
        }
        TransportTag::BULK_ACK => {
            if body.len() >= 17 {
                let stream_id = body[0];
                let chunks = u64::from_le_bytes(body[1..9].try_into().unwrap());
                let bytes_val = u64::from_le_bytes(body[9..17].try_into().unwrap());
                if let Some(conn) = state.connections.get(&conn_id) {
                    conn.resolve_bulk(stream_id, BulkOutcome::Delivered {
                        bytes_transferred: bytes_val,
                        duration: conn.connected_at.elapsed(),
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
                if let Some(conn) = state.connections.get(&conn_id) {
                    conn.resolve_bulk(stream_id, outcome);
                }
            }
        }
        TransportTag::BULK_CANCEL => {
            if !body.is_empty() {
                let stream_id = body[0];
                state.cancel_bulk_stream(conn_id, stream_id);
            }
        }
        TransportTag::SHUTDOWN => {
            let _ = transport_tx.try_send(encode_shutdown_ack());
        }
        TransportTag::SHUTDOWN_ACK => {}
        _ => {
            tracing::debug!(conn_id, tag, "unknown transport frame tag");
        }
    }
}

fn transition_and_notify<R: FrameRouter>(
    state: &Arc<ServerState>, router: &Arc<R>,
    conn_id: u64, new: ConnectionPhase,
) {
    if let Some(conn) = state.connections.get(&conn_id) {
        let old = conn.current_phase();
        if old != new {
            if let Ok(prev) = conn.transition(new) {
                drop(conn);
                router.on_connection_state_changed(state, conn_id, prev, new);
            }
        }
    }
}

async fn read_bulk_frame(
    reader: &mut (impl AsyncReadExt + Unpin),
) -> crate::error::IpcResult<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(crate::error::IpcError::ConnectionClosed);
        }
        Err(e) => return Err(crate::error::IpcError::Io(e)),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE as usize {
        return Err(crate::error::IpcError::FrameTooLarge {
            #[allow(clippy::cast_possible_truncation)]
            size: len as u32, max: MAX_FRAME_SIZE,
        });
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

#[cfg(unix)]
fn apply_socket_options(stream: &tokio::net::UnixStream, config: &IpcConfig) {
    #![allow(unsafe_code)]
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    if let Some(sndbuf) = config.uds_sndbuf {
        unsafe {
            let val = sndbuf as libc::c_int;
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                (&raw const val).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t);
        }
    }
    if let Some(rcvbuf) = config.uds_rcvbuf {
        unsafe {
            let val = rcvbuf as libc::c_int;
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&raw const val).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t);
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
