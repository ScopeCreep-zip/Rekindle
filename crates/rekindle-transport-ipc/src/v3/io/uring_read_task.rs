//! io_uring read task — dedicated OS thread reading frames from the socket
//! via io_uring, eliminating per-frame syscall overhead.
//!
//! # Thread constraint
//!
//! This task runs on a dedicated `std::thread::spawn` thread, NOT a tokio
//! worker thread. `blocking_send()` is used for ReadSignal delivery — it
//! parks via condvar, which is safe from a non-tokio thread. If this task
//! is ever migrated to `tokio::task::spawn` (an async task on a tokio
//! worker thread), `blocking_send` will panic. `spawn_blocking` is safe
//! because it runs on a dedicated blocking thread pool, not a worker thread.
//!
//! State machine per frame:
//!   1. Check EpochSignal for decoder key install or retirement
//!   2. Submit Read for envelope (32 bytes)
//!   3. CQE → accumulate bytes, resubmit on short read
//!   4. Full envelope → check EpochSignal again → verify EMAC + replay filter
//!   5. Submit Read for body (body_len bytes)
//!   6. CQE → accumulate bytes, resubmit on short read
//!   7. Full body → route to BulkReceiver or inline decode
//!   8. Send VerifiedFrame to control loop via blocking_send
//!   9. Goto 1
//!
//! Buffer lifetime: envelope_buf and body_buf are declared BEFORE the
//! IoUring ring. Rust drops in reverse declaration order — ring drops
//! first (cancelling in-flight SQEs), then buffers drop. No UAF.

#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use io_uring::{IoUring, opcode, types};

use crate::v3::bulk::counters::BulkCounters;
use crate::v3::bulk::recv::BulkReceiver;
use crate::v3::io::decode::{DecodeError, FrameDecoder};
use crate::v3::io::control_loop::{ReadSignal, VerifiedFrame};
use crate::v3::io::encode::FrameEncoder;
use crate::v3::io::epoch_signal::EpochSignal;
use crate::v3::io::read_task::{SessionOutcome, decode_error_to_outcome};
use crate::v3::wire::constants::ENVELOPE_LEN;
use crate::v3::wire::lane::Lane;

const TAG_ENVELOPE: u64 = 1;
const TAG_BODY: u64 = 2;
const TAG_SHUTDOWN: u64 = 3;

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

/// Drain the pending epoch signal and apply to both the decoder and
/// BulkReceiver. Called at every frame boundary — before envelope
/// verification and after body processing. Both live on the read task
/// thread and are updated together.
///
/// Retirement is slot overwrite: install(epoch=N) drops the previous
/// occupant of slot[N & 1] (epoch=N-2 keys). ZeroizeOnDrop fires.
/// No explicit retire call needed.
fn apply_epoch_signal(
    fd: RawFd,
    decoder: &mut FrameDecoder,
    bulk_receiver: &mut BulkReceiver,
    encoder: &crate::v3::io::encode::FrameEncoder,
    epoch_signal: &EpochSignal,
) {
    while let Some(data) = epoch_signal.drain() {
        let epoch = data.epoch;
        decoder.install_epoch_keys(epoch, data.decoder_keys.clone());
        bulk_receiver.install_epoch_keys(epoch, data.decoder_keys);
        let new_epoch = encoder.install_next_epoch(data.encoder_keys);
        debug_assert_eq!(epoch, new_epoch, "epoch signal epoch != encoder epoch after install");
        tracing::info!(fd, epoch, "uring read task: decoder + bulk receiver + encoder keys installed for new epoch");
    }
}

/// Run the uring read task. Blocks the calling std::thread until EOF,
/// error, replay detection, or shutdown eventfd signal.
pub fn run(
    fd: RawFd,
    decoder: FrameDecoder,
    signal_tx: tokio::sync::mpsc::Sender<ReadSignal>,
    bulk_receiver: BulkReceiver,
    bulk_threshold: u32,
    counters: Arc<BulkCounters>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    shutdown_eventfd: RawFd,
    epoch_signal: Arc<EpochSignal>,
    encoder: Arc<FrameEncoder>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(fd, decoder, signal_tx, bulk_receiver, bulk_threshold, counters, credit_guard, shutdown_eventfd, epoch_signal, encoder);
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
    signal_tx: tokio::sync::mpsc::Sender<ReadSignal>,
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
    let mut current_env_info: Option<crate::v3::codec::envelope::EnvelopeInfo> = None;
    let mut current_peer_epoch_advanced: bool = false;

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
    submit_read(&mut ring, fixed_fd, envelope_buf.as_mut_ptr(), ENVELOPE_LEN, TAG_ENVELOPE);
    ring.submit().expect("initial envelope submit failed");

    tracing::debug!(fd, "uring read task: entering main loop");

    loop {
        if ring.submit_and_wait(1).is_err() {
            tracing::debug!(fd, "uring read task: submit_and_wait failed, exiting");
            let _ = signal_tx.blocking_send(ReadSignal::Finished(SessionOutcome::SubstrateReadFailed {
                detail: "io_uring submit_and_wait failed".to_string(),
            }));
            break;
        }

        let mut should_exit = false;

        let (submitter, mut sq, mut cq) = ring.split();
        cq.sync();

        let overflow = cq.overflow();
        if overflow > 0 {
            tracing::error!(fd, overflow, "uring read task: CQ overflow — terminating");
            let _ = signal_tx.blocking_send(ReadSignal::Finished(SessionOutcome::SubstrateReadFailed {
                detail: format!("io_uring CQ overflow: {overflow} CQEs lost"),
            }));
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
                        detail: format!("io_uring read errno={errno} tag={tag} env_read={envelope_bytes_read} body_read={body_bytes_read} body_expected={body_expected}"),
                    }
                };
                let _ = signal_tx.blocking_send(ReadSignal::Finished(outcome));
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
                        let sqe = opcode::Read::new(fixed_fd, ptr, len as u32)
                            .build()
                            .user_data(TAG_ENVELOPE);
                        while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        continue;
                    }

                    // Check for epoch key install BEFORE every envelope verification.
                    // The control loop may have sent new decoder keys since the last
                    // check. Without this, a post-rotation frame can arrive before
                    // the decoder has the new epoch's keys installed.
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

                            let sqe = opcode::Read::new(
                                fixed_fd, body_buf.as_mut_ptr(), body_expected as u32
                            ).build().user_data(TAG_BODY);
                            while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        }
                        Err(DecodeError::ReplayDetected { session_seq }) => {
                            counters.replay_rejections.fetch_add(1, Ordering::Relaxed);
                            let _ = signal_tx.blocking_send(ReadSignal::Finished(
                                SessionOutcome::ReplayDetected { session_seq }
                            ));
                            should_exit = true;
                            break;
                        }
                        Err(ref e) => {
                            let epoch_flag = envelope_buf[2] & 0x01;
                            tracing::error!(
                                fd,
                                error = ?e,
                                envelope_hex = %hex::encode(&envelope_buf),
                                wire_version = envelope_buf[0],
                                lane = envelope_buf[1],
                                frame_epoch_flag = epoch_flag,
                                decoder_epoch = decoder.current_epoch(),
                                body_len_raw = u32::from_le_bytes([envelope_buf[4], envelope_buf[5], envelope_buf[6], envelope_buf[7]]),
                                session_seq_raw = u64::from_le_bytes([
                                    envelope_buf[8], envelope_buf[9], envelope_buf[10], envelope_buf[11],
                                    envelope_buf[12], envelope_buf[13], envelope_buf[14], envelope_buf[15],
                                ]),
                                "uring read task: envelope verification FAILED"
                            );
                            let _ = signal_tx.blocking_send(ReadSignal::Finished(decode_error_to_outcome(e.clone(), 0)));
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
                        let sqe = opcode::Read::new(fixed_fd, ptr, len)
                            .build()
                            .user_data(TAG_BODY);
                        while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                        continue;
                    }

                    let env_info = match current_env_info.take() {
                        Some(info) => info,
                        None => {
                            tracing::error!(fd, "uring read task: body CQE without envelope state");
                            let _ = signal_tx.blocking_send(ReadSignal::Finished(SessionOutcome::SubstrateReadFailed {
                                detail: "body CQE without envelope state".to_string(),
                            }));
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

                    if env_info.lane == Lane::Data && env_info.body_len >= bulk_threshold {
                        let frame_len = (ENVELOPE_LEN + body_expected) as u64;
                        if !credit_guard.try_reserve(frame_len) {
                            counters.memory_pressure_drops.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                session_seq = env_info.session_seq,
                                frame_len,
                                inflight = credit_guard.inflight(),
                                ceiling = credit_guard.ceiling(),
                                "CreditGuard: frame shed under memory pressure"
                            );
                        } else {
                            bulk_receiver.dispatch_from_parts(
                                env_info.session_seq, env_info.key_epoch(), &envelope_buf, &body_buf,
                            );
                        }
                    } else {
                        match decoder.decode_body(&envelope_buf, &env_info, &body_buf) {
                            Ok(decoded) => {
                                let frame = VerifiedFrame {
                                    envelope_bytes: envelope_buf,
                                    body: body_buf.clone(),
                                    envelope: decoded.envelope,
                                    header: decoded.header,
                                    plaintext: decoded.plaintext,
                                    peer_epoch_advanced: current_peer_epoch_advanced,
                                };
                                if signal_tx.blocking_send(ReadSignal::Frame(frame)).is_err() {
                                    should_exit = true;
                                    break;
                                }
                            }
                            Err(e) => {
                                if matches!(e, DecodeError::AeadVerificationFailed) {
                                    counters.aead_failures.fetch_add(1, Ordering::Relaxed);
                                }
                                let outcome = decode_error_to_outcome(e, env_info.session_seq);
                                let _ = signal_tx.blocking_send(ReadSignal::Finished(outcome));
                                should_exit = true;
                                break;
                            }
                        }
                    }

                    // Check for epoch signal after body processing, before next
                    // envelope read. Same helper as the pre-verify check — one
                    // function, no duplication.
                    apply_epoch_signal(fd, &mut decoder, &mut bulk_receiver, &encoder, &epoch_signal);

                    envelope_bytes_read = 0;
                    let sqe = opcode::Read::new(
                        fixed_fd, envelope_buf.as_mut_ptr(), ENVELOPE_LEN as u32
                    ).build().user_data(TAG_ENVELOPE);
                    while unsafe { sq.push(&sqe) }.is_err() { sq.sync(); }
                }
                TAG_SHUTDOWN => {
                    tracing::debug!(fd, "uring read task: shutdown eventfd signaled, exiting");
                    let _ = signal_tx.blocking_send(ReadSignal::Finished(SessionOutcome::Closed { peer_initiated: false }));
                    should_exit = true;
                    break;
                }
                _ => {
                    tracing::warn!(fd, tag, "uring read task: unknown CQE tag");
                }
            }
        }

        let dropped = sq.dropped();
        if dropped > 0 {
            tracing::error!(fd, dropped, "uring read task: kernel dropped SQEs");
            let _ = signal_tx.blocking_send(ReadSignal::Finished(SessionOutcome::SubstrateReadFailed {
                detail: format!("io_uring: kernel dropped {dropped} SQEs"),
            }));
            should_exit = true;
        }

        drop(sq);
        drop(cq);

        let _ = submitter.submit();

        if should_exit {
            break;
        }
    }

    tracing::debug!(fd, "uring read task: exiting cleanly");
}

fn submit_read(ring: &mut IoUring, fd: types::Fixed, ptr: *mut u8, len: usize, tag: u64) {
    let sqe = opcode::Read::new(fd, ptr, len as u32).build().user_data(tag);
    loop {
        if unsafe { ring.submission().push(&sqe) }.is_ok() {
            return;
        }
        if ring.submitter().squeue_wait().is_err() { return; }
    }
}
