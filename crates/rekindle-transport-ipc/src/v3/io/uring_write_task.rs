//! io_uring write task — dedicated OS thread, coalescing frame writer.
//!
//! ONE Send SQE in flight at a time. Frame boundaries are preserved
//! because each SQE is a contiguous byte sequence on the wire.
//! unix_stream_sendmsg loops internally — one sendmsg() = one contiguous
//! byte sequence regardless of kernel skb fragmentation.
//!
//! Throughput comes from coalescing: multiple small frames are drained
//! from channels and concatenated into a single buffer before submitting
//! one SQE. One syscall for N frames instead of N syscalls.
//!
//! Bulk frames (Vec<u8>) are sent individually — each is one SQE.
//! Cannot coalesce without copying 16 MiB.
//!
//! Priority: control_rx is ALWAYS drained first before any coalescing.
//! Channels are drained Control > Audit > Handoff > Data > Bulk.
//!
//! Only control_rx disconnect is fatal. Other channel disconnects are
//! non-fatal — those subsystems may shut down before the session ends.
//!
//! All write errors on SOCK_STREAM are fatal — the byte stream is
//! corrupted after any partial frame delivery.
//!
//! MSG_MORE does not exist on AF_UNIX. MSG_SPLICE_PAGES is kernel-internal.
//! Coalescing into a single buffer before sendmsg() is the only correct
//! approach for batching small frames on AF_UNIX SOCK_STREAM.
//!
//! CQ overflow semantics: with IORING_FEAT_NODROP (6.11+), CQ overflow
//! causes submit_and_wait to block until the application drains the CQ,
//! rather than silently dropping CQEs. With CQ=8 and 1 SQE in flight,
//! overflow is impossible.

#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::os::unix::io::RawFd;
use std::sync::Arc;

use io_uring::{IoUring, opcode, types};

use crate::v3::bulk::counters::BulkCounters;
use crate::v3::io::lane_channels::BulkFrame;

const WRITE_TOKEN: u64 = 1;

/// What we're currently sending.
enum Current {
    /// Coalesced small frames — multiple frames concatenated.
    Coalesced { buf: Vec<u8>, written: usize, frame_count: u64 },
    /// Single bulk frame — BulkFrame (Deref to &[u8]), cannot coalesce.
    Bulk { buf: BulkFrame, written: usize },
}

impl Current {
    /// Pointer to the next byte to send.
    /// SAFETY: caller must ensure written <= total length.
    fn ptr(&self) -> *const u8 {
        match self {
            Self::Coalesced { buf, written, .. } => {
                debug_assert!(*written <= buf.len());
                unsafe { buf.as_ptr().add(*written) }
            }
            Self::Bulk { buf, written } => {
                let bytes: &[u8] = buf;
                debug_assert!(*written <= bytes.len());
                unsafe { bytes.as_ptr().add(*written) }
            }
        }
    }

    fn remaining(&self) -> usize {
        match self {
            Self::Coalesced { buf, written, .. } => buf.len() - *written,
            Self::Bulk { buf, written } => { let b: &[u8] = buf; b.len() - *written },
        }
    }

    fn advance(&mut self, n: usize) {
        match self {
            Self::Coalesced { buf, written, .. } => {
                assert!(
                    *written + n <= buf.len(),
                    "advance({n}) would exceed buf len {} (written={})", buf.len(), *written,
                );
                *written += n;
            }
            Self::Bulk { buf, written } => {
                let len = { let b: &[u8] = buf; b.len() };
                assert!(
                    *written + n <= len,
                    "advance({n}) would exceed buf len {len} (written={})", *written,
                );
                *written += n;
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.remaining() == 0
    }

    fn record_sent(&self, counters: &BulkCounters) {
        match self {
            Self::Coalesced { buf, frame_count, .. } => {
                counters.record_sent(*frame_count, buf.len() as u64);
            }
            Self::Bulk { buf, .. } => {
                let b: &[u8] = buf;
                counters.record_sent(1, b.len() as u64);
            }
        }
    }

    /// Extract the coalesced buffer for reuse. Returns None for Bulk.
    fn take_coalesce_buf(self) -> Option<Vec<u8>> {
        match self {
            Self::Coalesced { mut buf, .. } => { buf.clear(); Some(buf) }
            Self::Bulk { .. } => None,
        }
    }
}

pub fn run(
    fd: RawFd,
    control_rx: crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: crossbeam::channel::Receiver<Vec<u8>>,
    bulk_rx: crossbeam::channel::Receiver<BulkFrame>,
    counters: Arc<BulkCounters>,
    error_tx: tokio::sync::mpsc::Sender<crate::v3::io::control_loop::WriteError>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(fd, control_rx, audit_rx, handoff_rx, data_rx, bulk_rx, counters, error_tx);
    }));
    if let Err(payload) = result {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            format!("unknown panic payload: {payload:?}")
        };
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!(
                "FATAL: uring write task panicked. Aborting.\n\
                 Panic message: {msg}\n\
                 Thread: {:?}\n",
                std::thread::current().name().unwrap_or("unnamed"),
            ),
        );
        std::process::abort();
    }
}

fn run_inner(
    fd: RawFd,
    control_rx: crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: crossbeam::channel::Receiver<Vec<u8>>,
    bulk_rx: crossbeam::channel::Receiver<BulkFrame>,
    counters: Arc<BulkCounters>,
    error_tx: tokio::sync::mpsc::Sender<crate::v3::io::control_loop::WriteError>,
) {
    let mut current: Option<Current> = None;
    // Reusable coalesce buffer — avoids heap allocation per iteration.
    let mut reuse_buf: Vec<u8> = Vec::with_capacity(64 * 1024);

    // Track which channels are still alive.
    let mut bulk_alive = true;
    let mut audit_alive = true;
    let mut handoff_alive = true;
    let mut data_alive = true;

    let mut ring = IoUring::builder()
        .setup_cqsize(8)
        .setup_coop_taskrun()
        .setup_single_issuer()
        .setup_defer_taskrun()
        .build(4)
        .unwrap_or_else(|_| {
            IoUring::builder()
                .setup_cqsize(8)
                .setup_coop_taskrun()
                .setup_single_issuer()
                .build(4)
                .expect("io_uring setup failed for write task")
        });

    assert!(
        ring.params().is_feature_submit_stable(),
        "io_uring: IORING_FEAT_SUBMIT_STABLE required (kernel >= 5.4)"
    );

    ring.submitter()
        .register_files(&[fd])
        .expect("io_uring register_files failed");
    let fixed_fd = types::Fixed(0);

    tracing::debug!(fd, "uring write task: entering main loop");

    loop {
        // ── Phase 1: Send current buffer ─────────────────────────────
        if let Some(ref cur) = current {
            let sqe = opcode::Send::new(fixed_fd, cur.ptr(), cur.remaining() as u32)
                .flags(libc::MSG_NOSIGNAL)
                .build()
                .user_data(WRITE_TOKEN);
            unsafe { ring.submission().push(&sqe) }
                .expect("SQ push failed with SQ size 4 and 1 in flight");

            if ring.submit_and_wait(1).is_err() {
                tracing::debug!(fd, "uring write task: submit_and_wait failed");
                break;
            }

            let mut should_exit = false;
            {
                let mut cq: io_uring::cqueue::CompletionQueue<'_, io_uring::cqueue::Entry> = ring.completion();
                cq.sync();
                for cqe in &mut cq {
                    let result = cqe.result();
                    // All write errors on SOCK_STREAM are fatal.
                    if result < 0 {
                        let errno = -result;
                        tracing::debug!(fd, errno, "uring write task: send failed — fatal");
                        let _ = error_tx.try_send(crate::v3::io::control_loop::WriteError::Io(
                            std::io::Error::from_raw_os_error(errno),
                        ));
                        current = None;
                        should_exit = true;
                    } else {
                        current.as_mut()
                            .expect("invariant: CQE arrived with no current buffer")
                            .advance(result as usize);
                    }
                }
            }

            if should_exit { break; }

            if let Some(ref cur) = current {
                if cur.is_complete() {
                    cur.record_sent(&counters);
                    // Reclaim coalesce buffer for reuse
                    if let Some(buf) = current.take().unwrap().take_coalesce_buf() {
                        reuse_buf = buf;
                    } else {
                        current = None;
                    }
                } else {
                    // Partial write — loop back, resubmit remainder.
                    // Do NOT check channels — complete current first.
                    continue;
                }
            }
        }

        // ── Phase 2: Coalesce small frames from all channels ─────────
        // Always drain control_rx first — highest priority.
        reuse_buf.clear();
        let mut frame_count: u64 = 0;

        drain_small(&control_rx, &mut reuse_buf, &mut frame_count);
        if audit_alive { drain_small(&audit_rx, &mut reuse_buf, &mut frame_count); }
        if handoff_alive { drain_small(&handoff_rx, &mut reuse_buf, &mut frame_count); }
        if data_alive { drain_small(&data_rx, &mut reuse_buf, &mut frame_count); }

        if !reuse_buf.is_empty() {
            // Move the buffer into Current, replace with fresh for next iteration
            let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
            current = Some(Current::Coalesced { buf, written: 0, frame_count });
            continue;
        }

        // No small frames — check bulk
        if bulk_alive {
            if let Ok(pooled) = bulk_rx.try_recv() {
                current = Some(Current::Bulk { buf: pooled, written: 0 });
                continue;
            }
        }

        // ── Phase 3: All channels empty — block ──────────────────────
        // Always check control_rx first before entering blocking select.
        if let Ok(buf) = control_rx.try_recv() {
            coalesce_all(&buf, &control_rx, &audit_rx, &handoff_rx, &data_rx,
                         audit_alive, handoff_alive, data_alive,
                         &mut reuse_buf, &mut frame_count);
            let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
            current = Some(Current::Coalesced { buf, written: 0, frame_count });
            continue;
        }

        let mut sel = crossbeam::channel::Select::new();
        let i_ctrl = sel.recv(&control_rx);
        let i_audit = if audit_alive { Some(sel.recv(&audit_rx)) } else { None };
        let i_handoff = if handoff_alive { Some(sel.recv(&handoff_rx)) } else { None };
        let i_data = if data_alive { Some(sel.recv(&data_rx)) } else { None };
        let i_bulk = if bulk_alive { Some(sel.recv(&bulk_rx)) } else { None };

        let oper = sel.select();
        let idx = oper.index();

        if idx == i_ctrl {
            match oper.recv(&control_rx) {
                Ok(buf) => {
                    frame_count = 0;
                    coalesce_all(&buf, &control_rx, &audit_rx, &handoff_rx, &data_rx,
                                 audit_alive, handoff_alive, data_alive,
                                 &mut reuse_buf, &mut frame_count);
                    let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
                    current = Some(Current::Coalesced { buf, written: 0, frame_count });
                }
                Err(_) => {
                    tracing::debug!(fd, "uring write task: control_rx disconnected — exiting");
                    break;
                }
            }
        } else if i_audit == Some(idx) {
            match oper.recv(&audit_rx) {
                Ok(buf) => {
                    frame_count = 0;
                    // Always drain control first for priority
                    coalesce_all(&buf, &control_rx, &audit_rx, &handoff_rx, &data_rx,
                                 audit_alive, handoff_alive, data_alive,
                                 &mut reuse_buf, &mut frame_count);
                    let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
                    current = Some(Current::Coalesced { buf, written: 0, frame_count });
                }
                Err(_) => { audit_alive = false; }
            }
        } else if i_handoff == Some(idx) {
            match oper.recv(&handoff_rx) {
                Ok(buf) => {
                    frame_count = 0;
                    coalesce_all(&buf, &control_rx, &audit_rx, &handoff_rx, &data_rx,
                                 audit_alive, handoff_alive, data_alive,
                                 &mut reuse_buf, &mut frame_count);
                    let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
                    current = Some(Current::Coalesced { buf, written: 0, frame_count });
                }
                Err(_) => { handoff_alive = false; }
            }
        } else if i_data == Some(idx) {
            match oper.recv(&data_rx) {
                Ok(buf) => {
                    frame_count = 0;
                    coalesce_all(&buf, &control_rx, &audit_rx, &handoff_rx, &data_rx,
                                 audit_alive, handoff_alive, data_alive,
                                 &mut reuse_buf, &mut frame_count);
                    let buf = std::mem::replace(&mut reuse_buf, Vec::with_capacity(64 * 1024));
                    current = Some(Current::Coalesced { buf, written: 0, frame_count });
                }
                Err(_) => { data_alive = false; }
            }
        } else if i_bulk == Some(idx) {
            match oper.recv(&bulk_rx) {
                Ok(pooled) => {
                    current = Some(Current::Bulk { buf: pooled, written: 0 });
                }
                Err(_) => { bulk_alive = false; }
            }
        }
    }

    // ── Shutdown ─────────────────────────────────────────────────────
    // Best-effort flush of current buffer. Partial writes are acceptable
    // on the shutdown path — the connection is being torn down.
    if let Some(ref cur) = current {
        if !cur.is_complete() {
            tracing::debug!(fd, remaining = cur.remaining(), "uring write task: shutdown — flushing remainder");
            let sqe = opcode::Send::new(fixed_fd, cur.ptr(), cur.remaining() as u32)
                .flags(libc::MSG_NOSIGNAL)
                .build()
                .user_data(WRITE_TOKEN);
            let _ = unsafe { ring.submission().push(&sqe) };
            if ring.submit_and_wait(1).is_ok() {
                let mut cq = ring.completion();
                cq.sync();
                for cqe in &mut cq {
                    tracing::trace!(fd, result = cqe.result(), "uring write task: shutdown CQE");
                }
            }
        }
    }
    // Drop current to return any held wire buf to the pool.
    drop(current);

    // Drain all remaining BulkFrames from the channel so their WireBufs
    // return to the pool. Without this, frames queued between the last
    // recv and the channel disconnect are dropped without returning
    // their wire bufs, leaking pool slots.
    while let Ok(frame) = bulk_rx.try_recv() {
        drop(frame); // BulkFrame::Pooled Drop returns WireBuf to pool
    }

    tracing::debug!(fd, "uring write task: exiting cleanly");
}

/// Drain all available frames from a small-frame channel into the buffer.
#[inline]
fn drain_small(
    rx: &crossbeam::channel::Receiver<Vec<u8>>,
    buf: &mut Vec<u8>,
    frame_count: &mut u64,
) {
    while let Ok(frame) = rx.try_recv() {
        buf.extend_from_slice(&frame);
        *frame_count += 1;
    }
}

/// Coalesce the initial frame with all available frames from all channels
/// in priority order: control > audit > handoff > data.
fn coalesce_all(
    initial: &[u8],
    control_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    audit_alive: bool,
    handoff_alive: bool,
    data_alive: bool,
    buf: &mut Vec<u8>,
    frame_count: &mut u64,
) {
    buf.clear();
    // Drain control first — always highest priority
    drain_small(control_rx, buf, frame_count);
    // Then the initial frame that woke us
    buf.extend_from_slice(initial);
    *frame_count += 1;
    // Then remaining channels by priority
    drain_small(control_rx, buf, frame_count); // control again in case more arrived
    if audit_alive { drain_small(audit_rx, buf, frame_count); }
    if handoff_alive { drain_small(handoff_rx, buf, frame_count); }
    if data_alive { drain_small(data_rx, buf, frame_count); }
}
