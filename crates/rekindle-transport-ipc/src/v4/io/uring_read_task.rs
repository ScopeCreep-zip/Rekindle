//! io_uring read task — dedicated OS thread reading frames from the socket
//! via io_uring, demultiplexing into per-lane channels.
//!
//! # Thread constraint
//!
//! This task runs on a dedicated `std::thread::spawn` thread, NOT a tokio
//! worker thread. It uses `try_send` with per-lane VecDeque overflow —
//! NEVER `blocking_send` (which would block the OS thread and starve all
//! lanes when one lane's channel fills — S-6 violation).
//!
//! # Lane demux
//!
//! After EMAC verification and body decode, the frame is routed to the
//! correct lane channel based on `envelope.lane`. Each lane channel has
//! independent backpressure — a full Handoff channel does NOT block
//! Control/Data/Audit frame delivery.
//!
//! # Per-lane overflow
//!
//! When `try_send` returns `Full`, the frame is buffered in a per-lane
//! `VecDeque`. The overflow is drained on every CQE batch. Overflow
//! growth is bounded by kernel socket buffer backpressure: when the
//! socket buffer fills, the sender blocks on `sock_wait_for_wmem`,
//! which stops new frames from arriving.
//!
//! Buffer lifetime: envelope_buf and body_buf are declared BEFORE the
//! IoUring ring. Rust drops in reverse declaration order — ring drops
//! first (cancelling in-flight SQEs), then buffers drop. No UAF.

#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::collections::VecDeque;
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use io_uring::{IoUring, opcode, types};

use crate::v4::bulk::counters::BulkCounters;
use crate::v4::bulk::recv::BulkReceiver;
use crate::v4::io::decode::{DecodeError, FrameDecoder};
use crate::v4::io::control_loop::{LaneInboundChannels, ReadSignal, VerifiedFrame};
use crate::v4::io::encode::FrameEncoder;
use crate::v4::io::epoch_signal::EpochSignal;
use crate::v4::io::read_task::{SessionOutcome, decode_error_to_outcome};
use crate::v4::wire::constants::ENVELOPE_LEN;
use crate::v4::wire::lane::Lane;

const TAG_ENVELOPE: u64 = 1;
const TAG_BODY: u64 = 2;
const TAG_SHUTDOWN: u64 = 3;

/// IORING_CQE_F_SOCK_NONEMPTY — kernel indicates more data available on socket.
/// Only set for OP_RECV / OP_RECVMSG CQEs, not OP_READ.
/// Value: 1 << 2 = 4. Reference: include/uapi/linux/io_uring.h
const CQE_F_SOCK_NONEMPTY: u32 = 1 << 2;

/// Overflow depth at which we log a warning.
const OVERFLOW_WARN_THRESHOLD: usize = 64;

/// Create an eventfd for cross-thread shutdown signaling.
pub fn create_shutdown_eventfd() -> RawFd {
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    assert!(fd >= 0, "eventfd creation failed: {}", std::io::Error::last_os_error());
    fd
}

/// Signal the shutdown eventfd. Wakes the read task's submit_and_wait.
pub fn signal_shutdown(eventfd: RawFd) {
    let val: u64 = 1;
    unsafe { libc::write(eventfd, (&raw const val).cast(), 8); }
}

/// Close the shutdown eventfd. Called after the read task thread is joined.
pub fn close_shutdown_eventfd(eventfd: RawFd) {
    unsafe { libc::close(eventfd); }
}

fn apply_epoch_signal(
    fd: RawFd,
    decoder: &mut FrameDecoder,
    bulk_receiver: &mut BulkReceiver,
    encoder: &FrameEncoder,
    epoch_signal: &EpochSignal,
) {
    while let Some(data) = epoch_signal.drain() {
        let epoch = data.epoch;
        decoder.install_epoch_keys(epoch, data.decoder_keys.clone());
        bulk_receiver.install_epoch_keys(epoch, data.decoder_keys);
        let new_epoch = encoder.install_next_epoch(data.encoder_keys);
        debug_assert_eq!(epoch, new_epoch, "epoch signal epoch != encoder epoch after install");
        tracing::info!(fd, epoch, "uring read task: epoch keys installed");
    }
}

/// Try to send a signal to the correct lane channel. On Full, buffer in overflow.
/// On Closed, return Err to signal lane task exit.
fn lane_try_send(
    lane_channels: &LaneInboundChannels,
    lane: Lane,
    signal: ReadSignal,
    overflow: &mut [VecDeque<ReadSignal>; 4],
) -> Result<(), ()> {
    let tx = match lane {
        Lane::Control => &lane_channels.control_tx,
        Lane::Data => &lane_channels.data_tx,
        Lane::Audit => &lane_channels.audit_tx,
        Lane::Handoff => &lane_channels.handoff_tx,
    };
    match tx.try_send(signal) {
        Ok(()) => Ok(()),
        Err(tokio::sync::mpsc::error::TrySendError::Full(s)) => {
            let idx = lane as usize;
            overflow[idx].push_back(s);
            if overflow[idx].len() == OVERFLOW_WARN_THRESHOLD {
                tracing::warn!(
                    ?lane,
                    overflow_depth = overflow[idx].len(),
                    "lane channel backpressure — buffering in read task"
                );
            }
            Ok(())
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            tracing::error!(?lane, "lane task exited — channel closed");
            Err(())
        }
    }
}

/// Drain per-lane overflow buffers into their channels.
fn drain_overflow(
    lane_channels: &LaneInboundChannels,
    overflow: &mut [VecDeque<ReadSignal>; 4],
) {
    for (idx, ovf) in overflow.iter_mut().enumerate() {
        let tx = match idx {
            0 => &lane_channels.control_tx,
            1 => &lane_channels.data_tx,
            2 => &lane_channels.audit_tx,
            3 => &lane_channels.handoff_tx,
            _ => unreachable!(),
        };
        while let Some(signal) = ovf.pop_front() {
            match tx.try_send(signal) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(s)) => {
                    ovf.push_front(s);
                    break; // still full, stop draining this lane
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // Lane task exited. Drop remaining overflow for this lane.
                    ovf.clear();
                    break;
                }
            }
        }
    }
}

/// Broadcast a shutdown signal to all lane channels. Best-effort.
/// Deliver the shutdown signal to the Control lane. Called immediately
/// before the read task exits — no more frames will be read after this.
///
/// Uses `blocking_send`, not `try_send`. The S-6 concern (blocking starves
/// other lanes) does not apply here because the read task is exiting —
/// it will never read another frame regardless. The `SessionOutcome`
/// must be delivered so the control loop knows WHY the session ended.
/// Silently dropping the shutdown reason is not acceptable.
///
/// If the Control channel is full (64 unprocessed frames), `blocking_send`
/// blocks until the control loop drains one slot. If the control loop is
/// dead, `blocking_send` blocks forever — but the read task was about to
/// exit anyway, and the kernel socket buffer will fill, and the peer will
/// detect the dead connection via its own heartbeat timeout.
fn broadcast_shutdown(
    lane_channels: &LaneInboundChannels,
    outcome: SessionOutcome,
) {
    let signal = ReadSignal::Finished(outcome);
    if lane_channels.control_tx.blocking_send(signal).is_err() {
        // Control lane already closed — shutdown already in progress.
        tracing::debug!("broadcast_shutdown: Control lane closed, shutdown reason not delivered");
    }
}

pub fn run(
    fd: RawFd,
    decoder: FrameDecoder,
    lane_channels: LaneInboundChannels,
    bulk_receiver: BulkReceiver,
    bulk_threshold: u32,
    counters: Arc<BulkCounters>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    shutdown_eventfd: RawFd,
    epoch_signal: Arc<EpochSignal>,
    encoder: Arc<FrameEncoder>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(fd, decoder, lane_channels, bulk_receiver, bulk_threshold, counters, credit_guard, shutdown_eventfd, epoch_signal, encoder);
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
                "FATAL: uring read task panicked. Aborting.\n\
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
    mut decoder: FrameDecoder,
    lane_channels: LaneInboundChannels,
    mut bulk_receiver: BulkReceiver,
    bulk_threshold: u32,
    counters: Arc<BulkCounters>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    shutdown_eventfd: RawFd,
    epoch_signal: Arc<EpochSignal>,
    encoder: Arc<FrameEncoder>,
) {
    // ── Buffers declared BEFORE ring ─────────────────────────────────
    let mut envelope_buf = [0u8; ENVELOPE_LEN];
    let mut envelope_bytes_read: usize = 0;
    let mut body_buf: Vec<u8> = Vec::new();
    let mut body_bytes_read: usize = 0;
    let mut body_expected: usize = 0;
    let mut current_env_info: Option<crate::v4::codec::envelope::EnvelopeInfo> = None;
    let mut current_peer_epoch_advanced: bool = false;

    // Per-lane overflow buffers. Index 0=Control, 1=Data, 2=Audit, 3=Handoff.
    // VecDeque::new() — no pre-allocation, expected to be empty under normal operation.
    let mut overflow: [VecDeque<ReadSignal>; 4] = [
        VecDeque::new(),
        VecDeque::new(),
        VecDeque::new(),
        VecDeque::new(),
    ];

    // ── Ring setup ───────────────────────────────────────────────────
    let mut ring = IoUring::builder()
        .setup_cqsize(128)
        .setup_coop_taskrun()
        .setup_single_issuer()
        .setup_submit_all()
        .setup_no_sqarray()
        .setup_defer_taskrun()
        .build(64)
        .unwrap_or_else(|_| {
            IoUring::builder()
                .setup_cqsize(128)
                .setup_coop_taskrun()
                .setup_single_issuer()
                .setup_submit_all()
                .build(64)
                .expect("io_uring setup failed for read task")
        });

    assert!(
        ring.params().is_feature_submit_stable(),
        "io_uring: IORING_FEAT_SUBMIT_STABLE required (kernel >= 5.4)"
    );

    ring.submitter()
        .register_files(&[fd])
        .expect("io_uring register_files failed for read task");
    let fixed_fd = types::Fixed(0);

    if let Err(e) = ring.register_ring_fd() {
        tracing::warn!(error = %e, "io_uring ring fd registration failed — fd table lookup not eliminated");
    }

    let mut max_workers = [0u32, 2u32];
    let _ = ring.submitter().register_iowq_max_workers(&mut max_workers);

    // Submit PollAdd on the shutdown eventfd for DEFER_TASKRUN wakeup.
    {
        let poll_sqe = opcode::PollAdd::new(types::Fd(shutdown_eventfd), libc::POLLIN as _)
            .build()
            .user_data(TAG_SHUTDOWN);
        unsafe { ring.submission().push(&poll_sqe) }
            .expect("failed to submit shutdown PollAdd SQE");
    }

    // Submit initial envelope read
    submit_recv(&mut ring, fixed_fd, envelope_buf.as_mut_ptr(), ENVELOPE_LEN, TAG_ENVELOPE);
    ring.submit().expect("initial envelope submit failed");

    tracing::debug!(fd, "uring read task: entering main loop (lane-sharded demux)");

    loop {
        if ring.submit_and_wait(1).is_err() {
            tracing::debug!(fd, "uring read task: submit_and_wait failed, exiting");
            broadcast_shutdown(&lane_channels, SessionOutcome::SubstrateReadFailed {
                detail: "io_uring submit_and_wait failed".to_string(),
            });
            break;
        }

        let mut should_exit = false;
        let mut sock_nonempty = false;

        let (submitter, mut sq, mut cq) = ring.split();
        cq.sync();

        let cq_overflow = cq.overflow();
        if cq_overflow > 0 {
            tracing::error!(fd, overflow = cq_overflow, "uring read task: CQ overflow — terminating");
            broadcast_shutdown(&lane_channels, SessionOutcome::SubstrateReadFailed {
                detail: format!("io_uring CQ overflow: {cq_overflow} CQEs lost"),
            });
            should_exit = true;
        }

        for cqe in cq.by_ref() {
            if should_exit { break; }
            let tag = cqe.user_data();
            let result = cqe.result();

            if result <= 0 && tag != TAG_SHUTDOWN {
                let outcome = if result == 0 {
                    tracing::debug!(fd, tag, "uring read task: result=0 (EOF), exiting");
                    SessionOutcome::ConnectionLost
                } else {
                    let errno = -result;
                    tracing::debug!(fd, tag, errno, envelope_bytes_read, body_bytes_read, body_expected,
                        "uring read task: read error, exiting");
                    SessionOutcome::SubstrateReadFailed {
                        detail: format!("io_uring read errno={errno} tag={tag}"),
                    }
                };
                broadcast_shutdown(&lane_channels, outcome);
                should_exit = true;
                break;
            }

            match tag {
                TAG_ENVELOPE => {
                    let bytes = result as usize;
                    envelope_bytes_read += bytes;
                    if envelope_bytes_read < ENVELOPE_LEN {
                        let ptr = unsafe { envelope_buf.as_mut_ptr().add(envelope_bytes_read) };
                        let len = ENVELOPE_LEN - envelope_bytes_read;
                        let sqe = opcode::Recv::new(fixed_fd, ptr, len as u32)
                            .build()
                            .user_data(TAG_ENVELOPE);
                        while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        continue;
                    }

                    apply_epoch_signal(fd, &mut decoder, &mut bulk_receiver, &encoder, &epoch_signal);

                    match decoder.verify_envelope(&envelope_buf) {
                        Ok((info, peer_advanced)) => {
                            body_expected = info.body_len as usize;
                            tracing::debug!(
                                fd, session_seq = info.session_seq, lane = ?info.lane,
                                body_len = body_expected, peer_epoch_advanced = peer_advanced,
                                "uring read task: envelope verified, reading body"
                            );
                            body_buf.clear();
                            body_buf.resize(body_expected, 0);
                            body_bytes_read = 0;
                            current_env_info = Some(info);
                            current_peer_epoch_advanced = peer_advanced;

                            let sqe = opcode::Recv::new(
                                fixed_fd, body_buf.as_mut_ptr(), body_expected as u32
                            ).build().user_data(TAG_BODY);
                            while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        }
                        Err(DecodeError::ReplayDetected { session_seq }) => {
                            counters.replay_rejections.fetch_add(1, Ordering::Relaxed);
                            broadcast_shutdown(&lane_channels, SessionOutcome::ReplayDetected { session_seq });
                            should_exit = true;
                            break;
                        }
                        Err(ref e) => {
                            tracing::error!(
                                fd,
                                error = ?e,
                                envelope_hex = %hex::encode(&envelope_buf),
                                "uring read task: envelope verification FAILED"
                            );
                            broadcast_shutdown(&lane_channels, decode_error_to_outcome(e.clone(), 0));
                            should_exit = true;
                            break;
                        }
                    }
                }
                TAG_BODY => {
                    let bytes = result as usize;
                    body_bytes_read += bytes;
                    if body_bytes_read < body_expected {
                        let ptr = unsafe { body_buf.as_mut_ptr().add(body_bytes_read) };
                        let len = (body_expected - body_bytes_read) as u32;
                        let sqe = opcode::Recv::new(fixed_fd, ptr, len)
                            .build()
                            .user_data(TAG_BODY);
                        while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        continue;
                    }

                    let env_info = match current_env_info.take() {
                        Some(info) => info,
                        None => {
                            tracing::error!(fd, "uring read task: body CQE without envelope state");
                            broadcast_shutdown(&lane_channels, SessionOutcome::SubstrateReadFailed {
                                detail: "body CQE without envelope state".to_string(),
                            });
                            should_exit = true;
                            break;
                        }
                    };
                    tracing::debug!(
                        fd, session_seq = env_info.session_seq, lane = ?env_info.lane,
                        body_len = body_expected,
                        "uring read task: body complete, frame total = {}",
                        ENVELOPE_LEN + body_expected,
                    );
                    counters.record_received(1, (ENVELOPE_LEN + body_expected) as u64);

                    if cqe.flags() & CQE_F_SOCK_NONEMPTY != 0 {
                        sock_nonempty = true;
                    }

                    if env_info.lane == Lane::Data && env_info.body_len >= bulk_threshold {
                        // Bulk path — dispatch to rayon pool, bypasses lane channels
                        let frame_len = (ENVELOPE_LEN + body_expected) as u64;
                        tracing::debug!(
                            session_seq = env_info.session_seq,
                            body_len = body_expected,
                            frame_len,
                            "uring read task: bulk threshold exceeded — dispatching to BulkReceiver"
                        );
                        if !credit_guard.try_reserve(frame_len) {
                            counters.memory_pressure_drops.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                session_seq = env_info.session_seq,
                                frame_len,
                                "uring read task: CreditGuard frame shed — memory pressure"
                            );
                        } else {
                            bulk_receiver.dispatch_from_parts(
                                env_info.session_seq, env_info.key_epoch(),
                                env_info,
                                &envelope_buf, &body_buf[..body_expected],
                            );
                        }
                    } else {
                        // Inline path — decode body and route to lane channel
                        match decoder.decode_body(&envelope_buf, &env_info, &body_buf) {
                            Ok(decoded) => {
                                let audit_envelope_hash = *blake3::hash(&envelope_buf).as_bytes();
                                let audit_header_hash = if env_info.lane == Lane::Data
                                    && body_expected >= crate::v4::wire::constants::STREAM_HEADER_LEN
                                {
                                    *blake3::hash(&body_buf[..crate::v4::wire::constants::STREAM_HEADER_LEN]).as_bytes()
                                } else {
                                    [0u8; 32]
                                };
                                let audit_ciphertext_hash = *blake3::hash(&body_buf[..body_expected]).as_bytes();

                                let mut retained_wire =
                                    Vec::with_capacity(ENVELOPE_LEN + body_expected);
                                retained_wire.extend_from_slice(&envelope_buf);
                                retained_wire.extend_from_slice(&body_buf[..body_expected]);

                                let frame = VerifiedFrame {
                                    envelope_bytes: envelope_buf,
                                    envelope: decoded.envelope,
                                    header: decoded.header,
                                    plaintext: decoded.plaintext,
                                    peer_epoch_advanced: current_peer_epoch_advanced,
                                    audit_envelope_hash,
                                    audit_header_hash,
                                    audit_ciphertext_hash,
                                    retained_wire,
                                };

                                // Route to the processing lane. Most frames go to
                                // the envelope's lane. Credit and Backpressure frames
                                // are rerouted from Control to Data (they modify Data
                                // lane state). See wire::lane::processing_lane.
                                let lane = crate::v4::wire::lane::processing_lane(env_info.lane, &frame.plaintext);
                                tracing::debug!(
                                    session_seq = env_info.session_seq,
                                    ?lane,
                                    plaintext_len = frame.plaintext.len(),
                                    "uring read task: inline frame decoded — dispatching to lane"
                                );
                                if lane_try_send(&lane_channels, lane, ReadSignal::Frame(frame), &mut overflow).is_err() {
                                    should_exit = true;
                                    break;
                                }
                            }
                            Err(e) => {
                                if matches!(e, DecodeError::AeadVerificationFailed) {
                                    counters.aead_failures.fetch_add(1, Ordering::Relaxed);
                                }
                                let outcome = decode_error_to_outcome(e, env_info.session_seq);
                                broadcast_shutdown(&lane_channels, outcome);
                                should_exit = true;
                                break;
                            }
                        }
                    }

                    apply_epoch_signal(fd, &mut decoder, &mut bulk_receiver, &encoder, &epoch_signal);

                    envelope_bytes_read = 0;
                    let sqe = opcode::Recv::new(
                        fixed_fd, envelope_buf.as_mut_ptr(), ENVELOPE_LEN as u32
                    ).build().user_data(TAG_ENVELOPE);
                    while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                }
                TAG_SHUTDOWN => {
                    tracing::debug!(fd, "uring read task: shutdown eventfd signaled, exiting");
                    broadcast_shutdown(&lane_channels, SessionOutcome::Closed { peer_initiated: false });
                    should_exit = true;
                    break;
                }
                _ => {
                    tracing::warn!(fd, tag, "uring read task: unknown CQE tag");
                }
            }
        }

        // After processing all CQEs in this batch, drain all per-lane overflows.
        drain_overflow(&lane_channels, &mut overflow);

        let dropped = sq.dropped();
        if dropped > 0 {
            tracing::error!(fd, dropped, "uring read task: kernel dropped SQEs");
            broadcast_shutdown(&lane_channels, SessionOutcome::SubstrateReadFailed {
                detail: format!("io_uring: kernel dropped {dropped} SQEs"),
            });
            should_exit = true;
        }

        drop(sq);
        drop(cq);

        if should_exit {
            let _ = submitter.submit();
            break;
        }

        if sock_nonempty {
            let _ = submitter.submit();
            continue;
        }
    }

    tracing::debug!(fd, "uring read task: exiting cleanly");
}

fn submit_recv(ring: &mut IoUring, fd: types::Fixed, ptr: *mut u8, len: usize, tag: u64) {
    let sqe = opcode::Recv::new(fd, ptr, len as u32).build().user_data(tag);
    loop {
        if unsafe { ring.submission().push(&sqe) }.is_ok() {
            return;
        }
        if ring.submitter().squeue_wait().is_err() { return; }
    }
}
