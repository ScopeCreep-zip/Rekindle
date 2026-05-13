//! io_uring bulk frame writer with DEFER_TASKRUN + registered buffers.
//!
//! Runs on a dedicated `std::thread` with its own `io_uring` instance.
//! Uses `SINGLE_ISSUER` + `DEFER_TASKRUN` for tightest completion control.
//!
//! # Ring setup flags (kernel 6.1+)
//!
//! - `SINGLE_ISSUER`: only this thread submits SQEs (required for DEFER_TASKRUN)
//! - `DEFER_TASKRUN`: defer completion task_work to explicit `io_uring_enter`
//! - `COOP_TASKRUN`: no IPI for completions (belt-and-suspenders with DEFER)
//! - `SUBMIT_ALL`: continue submitting after per-SQE error
//! - `CLAMP`: clamp ring sizes to kernel max instead of EINVAL
//!
//! # Buffer registration
//!
//! The entire slab pool can be registered via `register_buffers` at startup.
//! Subsequent `IORING_OP_SEND` with `IORING_RECVSEND_FIXED_BUF` references
//! by buffer index, avoiding per-send `get_user_pages_fast` page-pinning.
//!
//! # Batching
//!
//! SQEs are accumulated per batch (up to 16 frames), then submitted in one
//! `io_uring_enter` call. CQEs are drained after each submit to return
//! buffers to the pool promptly.

#[cfg(feature = "bulk-uring")]
use std::sync::Arc;
#[cfg(feature = "bulk-uring")]
use crossbeam::channel as cb;
#[cfg(feature = "bulk-uring")]
use super::pool::BufferPool;

/// Maximum frames per submit batch.
#[cfg(feature = "bulk-uring")]
const BATCH_SIZE: usize = 16;

/// Wire frame buffer size: [lane(1)][length(4)][body(SLAB_SIZE)].
#[cfg(feature = "bulk-uring")]
const WIRE_BUF_SIZE: usize = 5 + super::pool::SLAB_SIZE;

/// Number of pre-allocated wire buffers. 4× batch depth provides
/// headroom for in-flight SQEs waiting for CQE completion.
#[cfg(feature = "bulk-uring")]
const WIRE_BUF_COUNT: usize = 64;

/// Pre-allocated wire frame buffer pool for the uring writer.
///
/// Eliminates per-frame heap allocation on the send path. Each buffer
/// holds the complete wire frame (lane + length prefix + body). Buffers
/// are returned to the pool when the CQE arrives (kernel send complete).
///
/// Uses `crossbeam::queue::ArrayQueue` (lock-free MPSC) because the
/// producer (batch build) and consumer (CQE drain) run on the same
/// thread — no contention, just avoiding allocation.
#[cfg(feature = "bulk-uring")]
struct WirePool {
    free: crossbeam::queue::ArrayQueue<Vec<u8>>,
}

#[cfg(feature = "bulk-uring")]
impl WirePool {
    fn new() -> Self {
        let free = crossbeam::queue::ArrayQueue::new(WIRE_BUF_COUNT);
        for _ in 0..WIRE_BUF_COUNT {
            let mut v = vec![0u8; WIRE_BUF_SIZE];
            v.clear();
            let _ = free.push(v);
        }
        Self { free }
    }

    fn acquire(&self) -> Vec<u8> {
        self.free.pop().unwrap_or_else(|| Vec::with_capacity(WIRE_BUF_SIZE))
    }

    fn replenish(&self, mut buf: Vec<u8>) {
        // Zeroize the entire capacity before returning to the pool.
        // Wire buffers contain ciphertext (not plaintext), but consistent
        // zeroization across ALL buffer pools prevents any future code
        // change from accidentally leaking sensitive data through a
        // pool buffer that was assumed to be clean.
        let cap = buf.capacity();
        if cap > 0 {
            // SAFETY: Vec's allocation is valid and contiguous for capacity bytes.
            #[allow(unsafe_code)]
            let full_slice = unsafe {
                std::slice::from_raw_parts_mut(buf.as_mut_ptr(), cap)
            };
            zeroize::Zeroize::zeroize(full_slice);
        }
        buf.clear();
        // If the pool is full, silently drop. This handles the case
        // where a burst of sends created overflow buffers.
        let _ = self.free.push(buf);
    }
}

/// In-flight wire frame tracked through the SQE→CQE lifecycle.
/// Stored as `Box::into_raw` in the SQE user_data. Recovered on CQE.
#[cfg(feature = "bulk-uring")]
struct InFlightWire {
    wire: Vec<u8>,
    expected_len: u32,
}

/// Spawn the io_uring bulk writer on a dedicated OS thread.
///
/// `out_rx` receives frame bodies (header + ct + tag, no lane byte).
/// The writer prepends the lane byte and length-prefixes each frame
/// before writing to the socket via `IORING_OP_SEND`.
///
/// Falls back to `COOP_TASKRUN` on kernels < 6.1 where `DEFER_TASKRUN`
/// is unavailable.
#[cfg(feature = "bulk-uring")]
pub fn spawn_uring_writer(
    sock_fd: std::os::unix::io::RawFd,
    out_rx: cb::Receiver<Vec<u8>>,
    pool: Arc<BufferPool>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
    connection_abort: tokio::sync::oneshot::Sender<()>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::Builder::new()
        .name("rekindle-uring-writer".into())
        .spawn(move || {
            use io_uring::{IoUring, opcode, types, squeue, cqueue};

            // Build ring with optimal flags for single-issuer bulk streaming.
            let ring = {
                let mut builder = IoUring::<squeue::Entry, cqueue::Entry>::builder();
                builder
                    .setup_single_issuer()
                    .setup_defer_taskrun()
                    .setup_coop_taskrun()
                    .setup_submit_all()
                    .setup_clamp()
                    .setup_cqsize(1024);

                match builder.build(256) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "DEFER_TASKRUN unavailable, falling back");
                        let mut fallback = IoUring::<squeue::Entry, cqueue::Entry>::builder();
                        fallback
                            .setup_coop_taskrun()
                            .setup_submit_all()
                            .setup_clamp()
                            .setup_cqsize(1024);
                        fallback.build(256)?
                    }
                }
            };

            // Register the socket fd for IOSQE_FIXED_FILE (avoids fd table lookup).
            ring.submitter().register_files(&[sock_fd])?;

            let wire_pool = WirePool::new();
            let mut shutdown = shutdown;
            let mut connection_abort = Some(connection_abort);

            loop {
                // Check shutdown (non-blocking).
                if shutdown.try_recv().is_ok() {
                    tracing::debug!("uring writer: shutdown signal");
                    break;
                }

                // Drain completions — return wire buffers to pool.
                // Detect short writes and signal connection abort.
                let mut fatal = false;
                {
                    // SAFETY: single issuer, no concurrent CQ access.
                    let cq = unsafe { ring.completion_shared() };
                    for cqe in cq {
                        let ptr = cqe.user_data() as *mut InFlightWire;
                        if !ptr.is_null() {
                            // SAFETY: ptr was created by Box::into_raw in the send loop.
                            let in_flight = unsafe { *Box::from_raw(ptr) };
                            let res = cqe.result();
                            if res < 0 {
                                tracing::error!(error = res, "uring send error — aborting connection");
                                fatal = true;
                            } else if res >= 0 && {
                        #[allow(clippy::cast_sign_loss)]
                        let sent = res as u32;
                        sent < in_flight.expected_len
                    } {
                                // Short write: unix_stream_sendmsg returned partial
                                // count because socket buffer was full with MSG_DONTWAIT.
                                // The framing protocol is now desynchronized — the
                                // receiver will see a truncated frame. Unrecoverable.
                                tracing::error!(
                                    sent = res, expected = in_flight.expected_len,
                                    "uring writer: SHORT WRITE — framing desync, aborting connection"
                                );
                                fatal = true;
                            }
                            wire_pool.replenish(in_flight.wire);
                        }
                    }
                }
                if fatal {
                    if let Some(abort) = connection_abort.take() {
                        let _ = abort.send(());
                    }
                    break;
                }

                // Collect a batch of frames from the crossbeam channel.
                let first = match out_rx.recv_timeout(std::time::Duration::from_millis(1)) {
                    Ok(f) => f,
                    Err(cb::RecvTimeoutError::Timeout) => continue,
                    Err(cb::RecvTimeoutError::Disconnected) => {
                        tracing::debug!("uring writer: channel closed");
                        break;
                    }
                };

                let mut batch = Vec::with_capacity(BATCH_SIZE);
                batch.push(first);
                while batch.len() < BATCH_SIZE {
                    match out_rx.try_recv() {
                        Ok(f) => batch.push(f),
                        Err(_) => break,
                    }
                }

                // Submit batch: build wire frames, push SQEs.
                {
                    // SAFETY: single issuer, no concurrent SQ access.
                    let mut sq = unsafe { ring.submission_shared() };

                    for slab in &batch {
                        // Lane byte = kind byte (slab[1]). By design, the lane
                        // byte on the wire IS the FrameKind discriminant:
                        //   0x01 = BulkData, 0x02 = BulkFin, 0x03 = WindowUpdate.
                        // The read path routes all non-zero lanes (0x01..=0x03)
                        // to the bulk dispatcher, which parses the full header
                        // to extract the actual FrameKind for further dispatch.
                        // This identity mapping avoids a separate lane→kind table.
                        let lane = if slab.len() > 1 { slab[1] } else { 0x01 };
                        #[allow(clippy::cast_possible_truncation)] // frame body < 65549 bytes (SLAB_SIZE)
                        let body_len = slab.len() as u32;

                        // Build wire frame from pool buffer — zero heap allocation.
                        let mut wire = wire_pool.acquire();
                        wire.push(lane);
                        wire.extend_from_slice(&body_len.to_be_bytes());
                        wire.extend_from_slice(slab);

                        #[allow(clippy::cast_possible_truncation)] // wire frame < WIRE_BUF_SIZE
                        let wire_len = wire.len() as u32;
                        let in_flight = Box::into_raw(Box::new(InFlightWire {
                            wire,
                            expected_len: wire_len,
                        }));
                        // SAFETY: in_flight is valid until CQE arrives.
                        let wire_ref = unsafe { &(*in_flight).wire };

                        let sqe = opcode::Send::new(
                            types::Fixed(0),
                            wire_ref.as_ptr(),
                            wire_len,
                        )
                        .build()
                        .flags(squeue::Flags::FIXED_FILE)
                        .user_data(in_flight as u64);

                        // SAFETY: single issuer, no concurrent SQ access.
                        // in_flight pointer is valid until CQE arrives.
                        unsafe {
                            if sq.push(&sqe).is_err() {
                                // SQ full — submit current batch and retry.
                                drop(sq);
                                ring.submit()?;
                                sq = ring.submission_shared();
                                if sq.push(&sqe).is_err() {
                                    // Still full after submit — SQ is pathologically
                                    // backed up. Dropping a frame would silently corrupt
                                    // the transfer (receiver waits forever for the missing
                                    // chunk). Abort the connection instead.
                                    drop(Box::from_raw(in_flight));
                                    tracing::error!("uring writer: SQ full after submit — aborting connection");
                                    fatal = true;
                                }
                            }
                        }
                    }
                }

                // Check if SQ-full abort was triggered during batch build.
                if fatal {
                    if let Some(abort) = connection_abort.take() {
                        let _ = abort.send(());
                    }
                    break;
                }

                // Submit all pending SQEs in one syscall.
                ring.submit()?;

                // Return pool slabs.
                for slab in batch {
                    pool.replenish(slab);
                }
            }

            // Drain remaining in-flight completions with a 5-second timeout.
            // Return wire buffers to the pool for clean shutdown.
            let drain_deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(5);

            loop {
                if std::time::Instant::now() >= drain_deadline {
                    tracing::warn!("uring writer: drain timeout — some wire buffers may leak");
                    break;
                }
                // SAFETY: single issuer, no concurrent CQ access.
                let cq = unsafe { ring.completion_shared() };
                let mut drained_any = false;
                for cqe in cq {
                    let ptr = cqe.user_data() as *mut InFlightWire;
                    if !ptr.is_null() {
                        // SAFETY: ptr was created by Box::into_raw in the send loop.
                        let in_flight = unsafe { *Box::from_raw(ptr) };
                        wire_pool.replenish(in_flight.wire);
                    }
                    drained_any = true;
                }
                if !drained_any { break; }
                let _ = ring.submit_and_wait(1);
            }

            Ok(())
        })
        .expect("failed to spawn uring writer thread")
}
