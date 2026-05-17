//! Dedicated write task for a single IPC connection.
//!
//! Owns BufWriter<OwnedWriteHalf> and NoiseWriter. Receives frames
//! from four channels with biased priority ordering:
//! P0: transport-internal frames (Ack, Heartbeat, BulkAck/Nack, Shutdown)
//! P1: application responses (latency-sensitive)
//! P2: subscription events
//! P3: bulk data frames (batched via recv_many)
//!
//! write_all is NOT cancellation-safe inside select!. Therefore:
//! select! only receives frames. All writes happen AFTER the select
//! resolves, outside the cancellation window.
//!
//! Errors are propagated to the connection handler via error_tx.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc;

use crate::bulk::{BulkCounters, BufferPool};
use crate::envelope::SharedFrame;
use crate::frame::lane::LANE_CONTROL;
use crate::noise::NoiseWriter;
use crate::transport_frame::TransportTag;

/// Bulk data lane byte. Named constant — NEVER derived from payload content.
const LANE_BULK: u8 = 0x01;

/// Errors from the write loop propagated to the connection handler.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
    Encrypt(String),
}

/// What the select! loop decided to write.
enum WriteAction {
    Transport(Vec<u8>),
    Response(Bytes),
    Event(SharedFrame),
    Bulk(Vec<Vec<u8>>),
    Closed,
}

/// Run the write loop for a single connection.
pub async fn run(
    mut noise_writer: NoiseWriter,
    write_half: OwnedWriteHalf,
    mut transport_rx: mpsc::Receiver<Vec<u8>>,
    mut response_rx: mpsc::Receiver<Bytes>,
    mut event_rx: mpsc::Receiver<SharedFrame>,
    mut bulk_rx: mpsc::Receiver<Vec<u8>>,
    buffer_pool: Arc<BufferPool>,
    bulk_counters: Arc<BulkCounters>,
    error_tx: mpsc::Sender<WriteError>,
    conn_id: u64,
) {
    let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, write_half);
    let mut bulk_batch: Vec<Vec<u8>> = Vec::with_capacity(64);

    loop {
        // select! only decides WHAT to write. No write_all inside select!.
        // write_all is not cancellation-safe — partial writes are silently
        // discarded if another branch fires. All writes happen after select.
        let action = tokio::select! {
            biased;

            // P0: transport-internal (ack, heartbeat, shutdown).
            msg = transport_rx.recv() => {
                match msg {
                    Some(payload) => WriteAction::Transport(payload),
                    None => WriteAction::Closed,
                }
            }

            // P1: application responses.
            msg = response_rx.recv() => {
                match msg {
                    Some(payload) => WriteAction::Response(payload),
                    None => WriteAction::Closed,
                }
            }

            // P2: subscription events.
            msg = event_rx.recv() => {
                match msg {
                    Some(frame) => WriteAction::Event(frame),
                    None => WriteAction::Closed,
                }
            }

            // P3: bulk data frames. Batch via recv_many for write coalescing.
            // recv_many returns 0 when channel is closed (all senders dropped).
            n = bulk_rx.recv_many(&mut bulk_batch, 64) => {
                if n == 0 {
                    // Channel closed: all senders dropped. Shut down cleanly.
                    WriteAction::Closed
                } else {
                    let batch = std::mem::take(&mut bulk_batch);
                    WriteAction::Bulk(batch)
                }
            }
        };

        // ---- Execute the write OUTSIDE select! (cancellation-safe) ----

        let write_result: Result<(), WriteError> = match action {
            WriteAction::Transport(payload) => {
                write_noise_frame(&mut noise_writer, &mut writer, &payload).await
            }
            WriteAction::Response(payload) => {
                // Wrap with APP tag so client IO loop delivers to inbound_rx.
                // Without the tag, the client treats it as transport-internal
                // and never surfaces it to the application.
                let mut tagged = Vec::with_capacity(1 + payload.len());
                tagged.push(TransportTag::APP);
                tagged.extend_from_slice(&payload);
                write_noise_frame(&mut noise_writer, &mut writer, &tagged).await
            }
            WriteAction::Event(frame) => {
                let mut tagged = Vec::with_capacity(1 + frame.len());
                tagged.push(TransportTag::APP);
                tagged.extend_from_slice(&frame);
                write_noise_frame(&mut noise_writer, &mut writer, &tagged).await
            }
            WriteAction::Bulk(batch) => {
                let mut total_bytes = 0u64;
                let frame_count = batch.len() as u64;
                let mut result = Ok(());
                for frame_body in &batch {
                    total_bytes += frame_body.len() as u64;
                    // Lane byte is a named constant. NEVER derived from payload.
                    if let Err(e) = writer.write_all(&[LANE_BULK]).await {
                        result = Err(WriteError::Io(e));
                        break;
                    }
                    let len = (frame_body.len() as u32).to_le_bytes();
                    if let Err(e) = writer.write_all(&len).await {
                        result = Err(WriteError::Io(e));
                        break;
                    }
                    if let Err(e) = writer.write_all(frame_body).await {
                        result = Err(WriteError::Io(e));
                        break;
                    }
                }
                if result.is_ok() {
                    if let Err(e) = writer.flush().await {
                        result = Err(WriteError::Io(e));
                    }
                }
                if result.is_ok() {
                    bulk_counters.frames_sent.fetch_add(frame_count, Ordering::Relaxed);
                    bulk_counters.bytes_sent.fetch_add(total_bytes, Ordering::Relaxed);
                }
                for slab in batch {
                    buffer_pool.replenish(slab);
                }
                result
            }
            WriteAction::Closed => break,
        };

        if let Err(e) = write_result {
            tracing::debug!(conn_id, error = ?e, "write loop error");
            let _ = error_tx.try_send(e);
            return;
        }
    }

    tracing::debug!(conn_id, "write loop exiting");
}

/// Write a Noise-encrypted frame on lane 0x00 and flush.
/// Returns WriteError on failure.
async fn write_noise_frame(
    noise: &mut NoiseWriter,
    writer: &mut (impl AsyncWriteExt + Unpin),
    payload: &[u8],
) -> Result<(), WriteError> {
    writer
        .write_all(&[LANE_CONTROL])
        .await
        .map_err(WriteError::Io)?;
    noise
        .write_encrypted_frame(writer, payload)
        .await
        .map_err(|e| WriteError::Encrypt(e.to_string()))?;
    writer.flush().await.map_err(WriteError::Io)?;
    Ok(())
}
