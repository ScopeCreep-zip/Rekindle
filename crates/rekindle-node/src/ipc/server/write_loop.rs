//! Dedicated write task for a single IPC connection.
//!
//! Owns the `BufWriter<OwnedWriteHalf>` and the `NoiseWriter`. Receives
//! frames from three channels (response, event, bulk) and writes them
//! to the socket with biased priority ordering.
//!
//! The connection handler owns the read half and dispatches inbound
//! frames. This task owns the write half and serializes outbound frames.
//! Neither blocks the other.
//!
//! The bulk channel is `tokio::sync::mpsc` — bridged from the crossbeam
//! channel by a dedicated `std::thread` in connection.rs. All three
//! `select!` arms are native tokio async. No `block_in_place`, no polling.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc;

use crate::ipc::bulk::{BufferPool, BulkCounters};
use crate::ipc::message::SharedFrame;
use crate::ipc::noise::NoiseWriter;

use super::lane::LANE_CONTROL;

/// Run the write loop for a single connection.
///
/// Exits when all sender channels close or a write error occurs.
/// The caller (connection handler) aborts this task on disconnect.
pub async fn run(
    mut noise_writer: NoiseWriter,
    write_half: OwnedWriteHalf,
    mut response_rx: mpsc::Receiver<Bytes>,
    mut event_rx: mpsc::Receiver<SharedFrame>,
    mut bulk_out_rx: mpsc::Receiver<Vec<u8>>,
    buffer_pool: Arc<BufferPool>,
    bulk_counters: Arc<BulkCounters>,
    conn_id: u64,
) {
    let mut writer = tokio::io::BufWriter::with_capacity(8192, write_half);
    let mut response_batch: Vec<Bytes> = Vec::with_capacity(32);
    let mut event_batch: Vec<SharedFrame> = Vec::with_capacity(64);

    loop {
        tokio::select! {
            biased;

            // Priority 1: daemon responses — latency-sensitive.
            count = response_rx.recv_many(&mut response_batch, 32) => {
                if count == 0 { break; }
                for payload in response_batch.drain(..) {
                    if write_control_frame(&mut noise_writer, &mut writer, &payload).await.is_err() {
                        tracing::debug!(conn_id, "response write failed");
                        return;
                    }
                }
                if writer.flush().await.is_err() { return; }
            }

            // Priority 2: subscription events.
            count = event_rx.recv_many(&mut event_batch, 64) => {
                if count == 0 { break; }
                for frame in event_batch.drain(..) {
                    if write_control_frame(&mut noise_writer, &mut writer, &frame).await.is_err() {
                        tracing::debug!(conn_id, "event write failed");
                        return;
                    }
                }
                if writer.flush().await.is_err() { return; }
            }

            // Priority 3: bulk data frames from bridge thread.
            // recv() returns None when bulk_bridge_tx is dropped (bridge
            // exited or no bulk session). Zero CPU when idle — native
            // tokio waker, no polling, no block_in_place.
            frame = bulk_out_rx.recv() => {
                let Some(slab) = frame else { continue };
                let mut batch = vec![slab];
                while batch.len() < 16 {
                    match bulk_out_rx.try_recv() {
                        Ok(f) => batch.push(f),
                        Err(_) => break,
                    }
                }
                let mut total_bytes = 0u64;
                for frame_body in &batch {
                    total_bytes += frame_body.len() as u64;
                    let lane = if frame_body.len() > 1 { frame_body[1] } else { 0x01 };
                    if writer.write_all(&[lane]).await.is_err() {
                        tracing::debug!(conn_id, "bulk lane write failed");
                        for s in batch { buffer_pool.replenish(s); }
                        return;
                    }
                    if crate::ipc::framing::write_frame(&mut writer, frame_body).await.is_err() {
                        tracing::debug!(conn_id, "bulk frame write failed");
                        for s in batch { buffer_pool.replenish(s); }
                        return;
                    }
                }
                if writer.flush().await.is_err() {
                    for s in batch { buffer_pool.replenish(s); }
                    return;
                }
                let frame_count = batch.len() as u64;
                bulk_counters.frames_sent.fetch_add(frame_count, Ordering::Relaxed);
                bulk_counters.bytes_sent.fetch_add(total_bytes, Ordering::Relaxed);
                for slab in batch {
                    buffer_pool.replenish(slab);
                }
            }
        }
    }

    tracing::debug!(conn_id, "write loop exiting");
}

/// Write a control-plane frame: lane byte 0x00 + Noise-encrypted payload.
async fn write_control_frame<W: AsyncWriteExt + Unpin>(
    noise: &mut NoiseWriter,
    writer: &mut W,
    payload: &[u8],
) -> std::io::Result<()> {
    writer.write_all(&[LANE_CONTROL]).await?;
    noise.write_encrypted_frame(writer, payload).await
        .map_err(|e| std::io::Error::other(e.to_string()))
}
