//! Per-connection lifecycle: handshake, read loop, cleanup.
//!
//! The connection handler owns the read half of the socket and
//! dispatches inbound frames. The write half is owned by the
//! write_loop task (see `write_loop.rs`), which serializes all
//! outbound frames (response, event, bulk) with biased priority.
//!
//! # Bulk bridge lifecycle
//!
//! Rayon encrypt workers produce frames via `crossbeam::channel::Sender`
//! (blocking send, correct backpressure). The write loop consumes via
//! `tokio::sync::mpsc::Receiver` (native async, no polling). A dedicated
//! `std::thread` bridges the two: parks on `crossbeam::recv()` (zero CPU),
//! forwards via `mpsc::Sender::blocking_send()`.
//!
//! Shutdown ordering:
//! 1. Read loop exits (peer disconnect, error, shutdown signal)
//! 2. Drop `bulk_out_tx` (crossbeam sender) → bridge thread's `recv()`
//!    returns `Err(Disconnected)` → bridge exits → drops `bulk_bridge_tx`
//!    → write loop's `recv()` returns `None`
//! 3. Abort write loop task (handles case where response/event channels
//!    keep it alive after bulk channel closes)
//! 4. Join bridge thread (guaranteed to exit because sender was dropped
//!    in step 2)
//! 5. Cleanup connection state from ServerState

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use bytes::Bytes;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::ipc::bulk::{
    self, BulkCounters, BulkDispatcher, BufferPool,
    reassembly::Reassembler,
    dispatcher::DecryptedChunk,
};
use crate::ipc::message::{SecurityLevel, SharedFrame};
use crate::ipc::noise;
use crate::ipc::transport::PeerCredentials;

use super::lane::{self, LANE_CONTROL};
use super::routing;
use super::state::{ConnectionState, ServerState, TokenBucket};
use super::write_loop;

/// Handle a single client connection.
pub async fn handle_connection(
    state: Arc<ServerState>,
    conn_id: u64,
    stream: UnixStream,
    response_tx: mpsc::Sender<Bytes>,
    event_tx: mpsc::Sender<SharedFrame>,
    response_rx: mpsc::Receiver<Bytes>,
    event_rx: mpsc::Receiver<SharedFrame>,
    peer_creds: PeerCredentials,
    keypair: Arc<snow::Keypair>,
    encrypt_pool: Arc<rayon::ThreadPool>,
    buffer_pool: Arc<BufferPool>,
    bulk_counters: Arc<BulkCounters>,
) {
    // Tune socket buffer sizes for bulk throughput before splitting.
    // Increases SO_SNDBUF/SO_RCVBUF to allow more in-flight chunks.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        crate::ipc::bulk::hugepage_pool::tune_socket_buffers(stream.as_raw_fd());
    }

    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::with_capacity(8192, reader);
    let mut handshake_writer = tokio::io::BufWriter::with_capacity(8192, writer);

    let local_creds = PeerCredentials::local();
    let connected_at = Instant::now();

    // ── Noise IK handshake ──────────────────────────────────────
    let mut handshake_result = match noise::server_handshake(
        &mut reader,
        &mut handshake_writer,
        &keypair,
        &local_creds,
        &peer_creds,
    ).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(conn_id, peer_pid = peer_creds.pid, error = %e, "Noise handshake failed");
            return;
        }
    };

    // ── Derive bulk session ─────────────────────────────────────
    let bulk_session = handshake_result
        .take_handshake_hash()
        .map(|h| bulk::BulkSession::new(
            conn_id,
            bulk::kdf::derive_bulk_cipher(&h),
            Arc::clone(&buffer_pool),
        ));

    // ── Extract client pubkey ───────────────────────────────────
    let Some(key) = handshake_result.remote_static() else {
        tracing::error!(conn_id, "no remote static key after handshake");
        return;
    };
    let Ok(client_pubkey): std::result::Result<[u8; 32], _> = key.try_into() else {
        tracing::error!(conn_id, "client pubkey not 32 bytes");
        return;
    };

    // ── Registry lookup ─────────────────────────────────────────
    let (security_clearance, verified_name) = {
        let reg = state.registry.read();
        if let Some(identity) = reg.lookup(&client_pubkey) {
            let name: Option<Arc<str>> = reg.lookup_name(&client_pubkey).map(Arc::from);
            tracing::info!(conn_id, agent = name.as_deref().unwrap_or("unknown"),
                clearance = ?identity.security_level, "agent authenticated");
            (identity.security_level, name)
        } else {
            tracing::info!(conn_id, clearance = ?SecurityLevel::Open, "ephemeral client");
            (SecurityLevel::Open, None)
        }
    };

    // ── Register connection ─────────────────────────────────────
    let (cancel_bulk_tx, mut cancel_bulk_rx) = mpsc::channel::<u64>(16);
    let (bulk_send_tx, mut bulk_send_rx) = mpsc::channel::<(u8, Vec<u8>)>(16);
    let (bulk_deliver_tx, bulk_deliver_rx) = mpsc::channel::<(u8, Vec<u8>)>(64);
    state.connections.insert(conn_id, ConnectionState {
        agent_id: std::sync::OnceLock::new(),
        verified_name: verified_name.clone(),
        response_tx,
        event_tx,
        peer: peer_creds,
        security_clearance,
        connected_at,
        rate_limiter: TokenBucket::new(),
        bulk_nonce_counter: None,
        cancel_bulk_tx,
        bulk_send_tx,
        bulk_deliver_tx: bulk_deliver_tx.clone(),
        bulk_deliver_rx: parking_lot::Mutex::new(bulk_deliver_rx),
    });
    if let Some(ref name) = verified_name {
        state.name_to_conn.write().insert(name.to_string(), conn_id);
    }

    // ── Bulk plane setup ────────────────────────────────────────
    //
    // Receive side: reassembly channel (crossbeam, rayon→connection)
    let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
    let digest_algorithm = bulk_session.as_ref()
        .map(crate::ipc::bulk::session::BulkSession::digest_algorithm)
        .unwrap_or_default();
    let recv_pool = bulk::pool::BufferPool::new();
    let mut bulk_dispatcher = bulk_session.as_ref().map(|session| {
        BulkDispatcher::with_algorithm(
            Arc::clone(session.cipher()),
            Arc::clone(&encrypt_pool),
            reassembly_tx,
            session.digest_algorithm(),
            Arc::clone(&recv_pool),
        )
    });
    let mut reassembler = Reassembler::with_algorithm(1024, digest_algorithm);
    let mut accumulator = bulk::transfer::BulkTransferAccumulator::new(0);

    // Store the bulk session's nonce counter on ConnectionState so the
    // cancel handler can read the current nonce position for reassembler reset.
    if let Some(ref session) = bulk_session {
        if let Some(mut conn) = state.connections.get_mut(&conn_id) {
            conn.bulk_nonce_counter = Some(Arc::clone(session.nonce_counter()));
        }
    }

    // Send side: crossbeam (rayon workers) → bridge thread → tokio mpsc (write loop)
    //
    // bulk_out_tx: held here, passed to producers. Dropping it signals
    //   the bridge thread to exit (recv returns Disconnected).
    // bulk_bridge_tx: moved into the bridge thread. Dropping it signals
    //   the write loop that no more bulk frames are coming (recv returns None).
    let (bulk_out_tx, bulk_out_crossbeam_rx) = crossbeam::channel::bounded::<Vec<u8>>(64);
    let (bulk_bridge_tx, bulk_bridge_rx) = mpsc::channel::<Vec<u8>>(64);

    // Bridge thread: only spawned when bulk session exists.
    //
    // std::thread::spawn (not spawn_blocking) because this loop runs for
    // the lifetime of the connection. spawn_blocking is for bounded work
    // that finishes — a connection-lifetime loop would permanently occupy
    // a blocking pool slot without returning it to the pool's condvar loop.
    //
    // The thread parks on crossbeam::recv() with zero CPU when idle.
    // It exits when bulk_out_tx is dropped (recv returns Disconnected)
    // or when the write loop exits (blocking_send returns Err).
    let bridge_thread: Option<std::thread::JoinHandle<()>> = if bulk_session.is_some() {
        Some(
            std::thread::Builder::new()
                .name(format!("rekindle-bulk-bridge-{conn_id}"))
                .spawn(move || {
                    // bulk_out_crossbeam_rx and bulk_bridge_tx moved in.
                    // Both are dropped when this closure returns.
                    while let Ok(frame) = bulk_out_crossbeam_rx.recv() {
                        if bulk_bridge_tx.blocking_send(frame).is_err() {
                            // Write loop exited (mpsc Receiver dropped).
                            // Remaining crossbeam frames are discarded.
                            break;
                        }
                    }
                    // bulk_bridge_tx dropped here → write loop sees None
                })
                .expect("failed to spawn bulk bridge thread"),
        )
    } else {
        // No bulk session: close both bridge ends immediately.
        // bulk_out_crossbeam_rx dropped → no bridge thread, no resource.
        // bulk_bridge_tx dropped → write loop's bulk recv() returns None
        //   immediately on first poll. Zero overhead for non-bulk connections.
        drop(bulk_out_crossbeam_rx);
        drop(bulk_bridge_tx);
        None
    };

    // ── Spawn write loop ────────────────────────────────────────
    // tokio::io::BufWriter::into_inner() returns W directly (not Result).
    // Any buffered handshake bytes were already flushed by the handshake
    // functions via explicit flush() calls.
    let write_half = handshake_writer.into_inner();
    let write_handle = tokio::spawn(write_loop::run(
        handshake_result.writer,
        write_half,
        response_rx,
        event_rx,
        bulk_bridge_rx,
        Arc::clone(&buffer_pool),
        Arc::clone(&bulk_counters),
        conn_id,
    ));

    // ── Read loop ───────────────────────────────────────────────
    let mut noise_reader = handshake_result.reader;

    loop {
        match lane::read_lane_byte(&mut reader).await {
            Ok(LANE_CONTROL) => {
                match noise_reader.read_encrypted_frame(&mut reader).await {
                    Ok(payload) => {
                        routing::route_frame(&state, conn_id, payload);
                    }
                    Err(e) => {
                        tracing::info!(conn_id, session_ms = %connected_at.elapsed().as_millis(), error = %e, "disconnected");
                        break;
                    }
                }
            }
            Ok(bulk_lane) if lane::is_bulk_lane(bulk_lane) => {
                match read_bulk_frame_pooled(&mut reader, &recv_pool).await {
                    Ok(frame_body) => {
                        let frame_len = frame_body.len() as u64;
                        if let Some(ref mut dispatcher) = bulk_dispatcher {
                            if dispatcher.dispatch(frame_body).is_ok() {
                                bulk_counters.frames_received.fetch_add(1, Ordering::Relaxed);
                                bulk_counters.bytes_received.fetch_add(frame_len, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::info!(conn_id, error = %e, "disconnected (bulk read)");
                        break;
                    }
                }
            }
            Ok(unknown) => {
                tracing::warn!(conn_id, lane = unknown, "unknown lane byte");
                break;
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::UnexpectedEof {
                    tracing::info!(conn_id, error = %e, "disconnected (read error)");
                }
                break;
            }
        }

        // Drain reassembled chunks into the accumulator.
        while let Ok(chunk) = reassembly_rx.try_recv() {
            match reassembler.process(chunk) {
                Ok(delivered) => {
                    for r in delivered {
                        let stream_id = r.stream_id;
                        let chunk_index = r.chunk_index;
                        let is_last = r.is_last;
                        let bytes = r.plaintext.len();
                        tracing::trace!(conn_id, stream_id, chunk_index, bytes, is_last, "chunk reassembled");
                        if let Some(complete) = accumulator.push(&r) {
                            tracing::info!(conn_id, stream_id, bytes = complete.len(), "bulk transfer complete");
                            let _ = bulk_deliver_tx.try_send((stream_id, complete));
                            accumulator = bulk::transfer::BulkTransferAccumulator::new(0);
                        }
                    }
                }
                Err(e) => tracing::warn!(conn_id, error = %e, "reassembly error"),
            }
        }

        // Poll send command channel — dispatch requests us to send data.
        while let Ok((stream_id, payload)) = bulk_send_rx.try_recv() {
            if let Some(ref session) = bulk_session {
                tracing::info!(conn_id, stream_id, bytes = payload.len(), "bulk send initiated");
                bulk::transfer::send_payload(
                    &encrypt_pool, session.cipher(), session.nonce_counter(),
                    &buffer_pool, bulk_out_tx.clone(), stream_id,
                    &payload, session.digest_algorithm(),
                );
            }
        }

        // Poll cancel channel — reset reassembler if a transfer was cancelled.
        while let Ok(next_nonce) = cancel_bulk_rx.try_recv() {
            reassembler.reset(next_nonce);
            accumulator = bulk::transfer::BulkTransferAccumulator::new(0);
            tracing::info!(conn_id, next_nonce, "reassembler reset after transfer cancel");
        }
    }

    // ── Teardown (ordered) ──────────────────────────────────────
    //
    // Step 1: Drop the crossbeam sender. This unblocks the bridge
    //   thread's recv() call, causing it to exit its loop and drop
    //   bulk_bridge_tx, which closes the mpsc channel to the write loop.
    drop(bulk_out_tx);

    // Step 2: Abort the write loop. It may still be alive servicing
    //   response/event channels even after the bulk mpsc closes.
    write_handle.abort();
    let _ = write_handle.await;

    // Step 3: Join the bridge thread. It is guaranteed to exit because
    //   bulk_out_tx was dropped in step 1 (recv returns Disconnected).
    //   If the write loop exited first (step 2), blocking_send already
    //   returned Err and the bridge exited — join returns immediately.
    if let Some(handle) = bridge_thread {
        let _ = handle.join();
    }

    // Step 4: BulkSession dropped here (local variable). Drop impl
    //   logs nonce count and session duration.
    drop(bulk_session);

    // Step 5: Remove from routing tables + log.
    cleanup_connection(&state, conn_id, verified_name.as_ref());
}

/// Read a bulk frame directly into a pool slab.
///
/// Reads the 4-byte BE length prefix, acquires a slab from the receive
/// pool, reads the frame body into the slab. The slab travels through
/// the dispatcher → rayon decrypt → reassembly pipeline and returns
/// to the pool when the application drops the `ReassembledChunk`.
async fn read_bulk_frame_pooled<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
    pool: &Arc<bulk::pool::BufferPool>,
) -> crate::ipc::error::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(crate::ipc::error::IpcError::ConnectionClosed);
        }
        Err(e) => return Err(crate::ipc::error::IpcError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > crate::ipc::framing::MAX_FRAME_SIZE as usize {
        return Err(crate::ipc::error::IpcError::FrameTooLarge {
            #[allow(clippy::cast_possible_truncation)]
            size: len as u32,
            max: crate::ipc::framing::MAX_FRAME_SIZE,
        });
    }
    let mut slab = pool.acquire();
    slab.resize(len, 0);
    reader.read_exact(&mut slab[..len]).await?;
    Ok(slab)
}

fn cleanup_connection(state: &ServerState, conn_id: u64, verified_name: Option<&Arc<str>>) {
    if let Some((_, conn)) = state.connections.remove(&conn_id) {
        #[allow(clippy::cast_possible_truncation)]
        let duration = conn.connected_at.elapsed().as_millis() as u64;
        tracing::info!(conn_id, agent = conn.verified_name.as_deref().unwrap_or("ephemeral"),
            peer_pid = conn.peer.pid, session_ms = duration, "connection cleanup");
    }
    state.event_router.remove_connection(conn_id);
    state.pending_requests.lock().remove_by_conn_id(conn_id);
    if let Some(name) = verified_name {
        state.name_to_conn.write().remove(name.as_ref());
    }
}
