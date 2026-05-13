//! io_uring-backed bulk frame writer on a dedicated OS thread.
//!
//! Runs a `tokio_uring` current-thread runtime on a dedicated
//! `std::thread`. Communicates with the main Tokio runtime via a
//! crossbeam channel (runtime-agnostic, sync sender from rayon workers).
//!
//! Each frame body arrives via crossbeam. The writer prepends the lane
//! byte then writes via `write_all` which submits `IORING_OP_WRITE`
//! SQEs. The SQ is flushed on thread park (tokio-uring's
//! `on_thread_park` hook), providing natural batching.

#[cfg(feature = "bulk-uring")]
use std::sync::Arc;
#[cfg(feature = "bulk-uring")]
use crossbeam::channel as cb;
#[cfg(feature = "bulk-uring")]
use super::pool::BufferPool;

/// Total bytes needed for fixed buffer registration.
#[cfg(feature = "bulk-uring")]
pub const FIXED_BUF_COUNT: usize = 64;
#[cfg(feature = "bulk-uring")]
pub const FIXED_BUF_SIZE: usize = super::pool::SLAB_SIZE;
#[cfg(feature = "bulk-uring")]
pub const FIXED_BUF_TOTAL_BYTES: usize = FIXED_BUF_COUNT * FIXED_BUF_SIZE;

/// Spawn the io_uring bulk writer on a dedicated OS thread.
///
/// `out_rx` receives frame bodies (header + ct + tag, no lane byte).
/// The writer prepends the lane byte and length-prefixes each frame
/// before writing to the socket.
#[cfg(feature = "bulk-uring")]
pub fn spawn_uring_writer(
    sock_fd: std::os::unix::io::RawFd,
    out_rx: cb::Receiver<Vec<u8>>,
    pool: Arc<BufferPool>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::Builder::new()
        .name("rekindle-uring-writer".into())
        .spawn(move || {
            tokio_uring::start(async move {
                // SAFETY: sock_fd is a valid open Unix stream socket fd.
                // The caller transferred ownership by not closing the fd.
                #[allow(unsafe_code)]
                let stream = unsafe {
                    tokio_uring::net::UnixStream::from_raw_fd(sock_fd)
                };

                let mut shutdown = shutdown;

                loop {
                    if shutdown.try_recv().is_ok() {
                        tracing::debug!("uring writer: shutdown");
                        return Ok(());
                    }

                    // Block on the crossbeam channel with a timeout
                    // to periodically check the shutdown signal.
                    let frame_body = match tokio::task::block_in_place(|| {
                        out_rx.recv_timeout(std::time::Duration::from_millis(5))
                    }) {
                        Ok(f) => f,
                        Err(cb::RecvTimeoutError::Timeout) => continue,
                        Err(cb::RecvTimeoutError::Disconnected) => {
                            tracing::debug!("uring writer: channel closed");
                            return Ok(());
                        }
                    };

                    // Prepend lane byte (kind is at offset 1 of the body).
                    let lane = if frame_body.len() > 1 { frame_body[1] } else { 0x01 };

                    // Build the wire frame: [lane][4B length BE][body]
                    let body_len = frame_body.len() as u32;
                    let mut wire_frame = Vec::with_capacity(1 + 4 + frame_body.len());
                    wire_frame.push(lane);
                    wire_frame.extend_from_slice(&body_len.to_be_bytes());
                    wire_frame.extend_from_slice(&frame_body);

                    // Return the original slab to the pool immediately.
                    pool.replenish(frame_body);

                    // Write the complete wire frame.
                    let (result, _) = stream.write_all(wire_frame).await;
                    result?;
                }
            })
        })
        .expect("failed to spawn uring writer thread")
}

#[cfg(feature = "bulk-uring")]
use std::os::unix::io::FromRawFd;
