//! Standalone bulk frame writer for dedicated-socket transports (TCP).
//!
//! This writer is used when bulk data has its own dedicated socket
//! (e.g., TCP cross-node transport). For the shared-socket UDS
//! architecture, the `write_loop` module in the server handles bulk
//! writes alongside control-plane writes.
//!
//! Each slab arriving from the channel is a frame body:
//! `[stream_id][kind][nonce][ciphertext][tag]`
//!
//! The writer prepends the lane byte and uses `write_frame` to add
//! the 4-byte length prefix before writing to the socket.
//!
//! # Batching
//!
//! Up to 16 frames are accumulated before each writev call:
//! - 16 frames × ~65.5 KiB ≈ 1 MiB per syscall
//! - At 10 Gbps: ~1,250 writev calls/sec
//! - Linux `IOV_MAX` = 1024, so 16 is well within limits
//!
//! # Pool return
//!
//! After each writev batch, the writer returns all `Vec<u8>` slabs
//! to the `BufferPool` via `pool.replenish()`. This closes the
//! acquire→encrypt→send→write→return lifecycle with zero allocation.

use std::sync::Arc;
use crossbeam::channel as cb;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::OwnedWriteHalf;

use super::pool::BufferPool;

/// Maximum frames per writev batch.
const BATCH_SIZE: usize = 16;

/// Timeout for draining the channel before yielding to Tokio.
const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(5);

/// Bulk writer task. Runs as a Tokio spawned task.
///
/// `sock` is the write half of the connection's Unix socket.
/// `out_rx` receives frame body `Vec<u8>` slabs from rayon encrypt
///   workers. The writer prepends the lane byte and length prefix.
/// `pool` is the buffer pool for returning slabs after writes.
/// `shutdown` signals graceful shutdown.
pub async fn bulk_writer_task(
    mut sock: OwnedWriteHalf,
    out_rx: cb::Receiver<Vec<u8>>,
    pool: Arc<BufferPool>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    let mut batch: Vec<Vec<u8>> = Vec::with_capacity(BATCH_SIZE);

    loop {
        // Check shutdown (non-blocking, highest priority).
        if shutdown.try_recv().is_ok() {
            tracing::debug!("bulk writer: shutdown");
            return Ok(());
        }

        // Wait for the first frame. block_in_place hands the Tokio
        // worker's run queue to another thread while we block on
        // the crossbeam recv_timeout.
        let first = tokio::task::block_in_place(|| out_rx.recv_timeout(DRAIN_TIMEOUT));
        match first {
            Ok(frame) => batch.push(frame),
            Err(cb::RecvTimeoutError::Timeout) => continue,
            Err(cb::RecvTimeoutError::Disconnected) => {
                tracing::debug!("bulk writer: channel closed");
                return Ok(());
            }
        }

        // Drain additional frames without blocking (up to BATCH_SIZE - 1).
        while batch.len() < BATCH_SIZE {
            match out_rx.try_recv() {
                Ok(frame) => batch.push(frame),
                Err(_) => break,
            }
        }

        // Each frame body needs a lane byte and length prefix.
        // Write each frame individually: [lane][write_frame(body)].
        for frame_body in &batch {
            let lane = if frame_body.len() > 1 { frame_body[1] } else { 0x01 };
            sock.write_all(&[lane]).await?;
            crate::ipc::framing::write_frame(&mut sock, frame_body).await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        sock.flush().await?;

        // Return all slabs to the pool.
        for slab in batch.drain(..) {
            pool.replenish(slab);
        }
    }
}
