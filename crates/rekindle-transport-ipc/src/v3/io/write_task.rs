//! Blocking write task — dedicated OS thread, coalescing frame writer.
//!
//! Non-Linux fallback for the io_uring write task. Same architecture:
//! dedicated std::thread consuming crossbeam lane receivers directly.
//! Uses std::io::Write + writev via IoSlice instead of io_uring SQEs.
//!
//! Five channels, biased priority: Control > Audit > Handoff > Data > Bulk.
//! Small frames are collected into a Vec of references and written via
//! write_vectored (writev(2)) — zero intermediate copy. Bulk frames are
//! written individually.
//!
//! The stream is set to blocking mode at startup. tokio's into_std()
//! leaves the fd nonblocking; write_all on a nonblocking stream returns
//! WouldBlock when the kernel buffer fills, which is a spurious fatal
//! error. set_nonblocking(false) is mandatory.
//!
//! Only control_rx disconnect is fatal. Other channel disconnects are
//! non-fatal — those subsystems may shut down before the session ends.
//! All write errors on SOCK_STREAM are fatal.

#[cfg(not(target_os = "linux"))]

use std::io::{IoSlice, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use crate::v3::bulk::counters::BulkCounters;
use crate::v3::io::lane_channels::BulkFrame;

/// Maximum bytes to coalesce per write_vectored call.
/// Caps drain_small to prevent unbounded heap growth under burst.
const MAX_COALESCE_BYTES: usize = 256 * 1024;

pub fn run(
    mut stream: UnixStream,
    control_rx: crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: crossbeam::channel::Receiver<Vec<u8>>,
    bulk_rx: crossbeam::channel::Receiver<BulkFrame>,
    counters: Arc<BulkCounters>,
    error_tx: tokio::sync::mpsc::Sender<crate::v3::io::control_loop::WriteError>,
) {
    // Issue 1: tokio's into_std() leaves the fd nonblocking. write_all on
    // a nonblocking stream returns WouldBlock when the kernel buffer fills.
    // Set blocking mode before any write.
    stream.set_nonblocking(false).expect("write task: set blocking mode");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_inner(
            &mut stream, &control_rx, &audit_rx, &handoff_rx,
            &data_rx, &bulk_rx, &counters, &error_tx,
        );
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
                "FATAL: write task panicked. Aborting.\n\
                 Panic message: {msg}\n\
                 Thread: {:?}\n",
                std::thread::current().name().unwrap_or("unnamed"),
            ),
        );
        std::process::abort();
    }
}

/// Report a write error. If the error channel is full, abort — a write
/// error that cannot be reported is a fatal unrecoverable condition.
fn report_error(
    error_tx: &tokio::sync::mpsc::Sender<crate::v3::io::control_loop::WriteError>,
    e: std::io::Error,
) {
    if error_tx.try_send(crate::v3::io::control_loop::WriteError::Io(e)).is_err() {
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!("FATAL: write error channel full — cannot report write failure. Aborting.\n"),
        );
        std::process::abort();
    }
}

/// Collect frames from a channel into the batch vec, up to MAX_COALESCE_BYTES total.
#[inline]
fn drain_small(
    rx: &crossbeam::channel::Receiver<Vec<u8>>,
    batch: &mut Vec<Vec<u8>>,
    total_bytes: &mut usize,
) {
    while *total_bytes < MAX_COALESCE_BYTES {
        match rx.try_recv() {
            Ok(frame) => {
                *total_bytes += frame.len();
                batch.push(frame);
            }
            Err(_) => break,
        }
    }
}

/// Write all frames via writev (write_vectored). Handles partial writes.
fn write_batch(stream: &mut UnixStream, batch: &[Vec<u8>]) -> Result<(), std::io::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let slices: Vec<IoSlice<'_>> = batch.iter().map(|f| IoSlice::new(f)).collect();
    let total: usize = slices.iter().map(|s| s.len()).sum();
    if total == 0 {
        return Ok(());
    }

    let n = stream.write_vectored(&slices)?;
    if n >= total {
        return Ok(());
    }

    // Partial write — fall back to write_all for remaining buffers.
    let mut written = n;
    for buf in batch {
        if written >= buf.len() {
            written -= buf.len();
            continue;
        }
        stream.write_all(&buf[written..])?;
        written = 0;
    }
    Ok(())
}

/// Drain all priority channels into batch, respecting priority order and byte cap.
fn coalesce_all(
    control_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    audit_alive: bool,
    handoff_alive: bool,
    data_alive: bool,
    batch: &mut Vec<Vec<u8>>,
    total_bytes: &mut usize,
) {
    drain_small(control_rx, batch, total_bytes);
    if audit_alive { drain_small(audit_rx, batch, total_bytes); }
    if handoff_alive { drain_small(handoff_rx, batch, total_bytes); }
    if data_alive { drain_small(data_rx, batch, total_bytes); }
}

fn run_inner(
    stream: &mut UnixStream,
    control_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    audit_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    handoff_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    data_rx: &crossbeam::channel::Receiver<Vec<u8>>,
    bulk_rx: &crossbeam::channel::Receiver<BulkFrame>,
    counters: &BulkCounters,
    error_tx: &tokio::sync::mpsc::Sender<crate::v3::io::control_loop::WriteError>,
) {
    let mut bulk_alive = true;
    let mut audit_alive = true;
    let mut handoff_alive = true;
    let mut data_alive = true;
    let mut batch: Vec<Vec<u8>> = Vec::with_capacity(64);

    loop {
        // Phase 1: Drain small frames from all channels (priority order)
        batch.clear();
        let mut total_bytes: usize = 0;
        coalesce_all(control_rx, audit_rx, handoff_rx, data_rx,
                     audit_alive, handoff_alive, data_alive,
                     &mut batch, &mut total_bytes);

        if !batch.is_empty() {
            let count = batch.len() as u64;
            if let Err(e) = write_batch(stream, &batch) {
                report_error(error_tx, e);
                return;
            }
            counters.record_sent(count, total_bytes as u64);
            // Issue 8: shrink batch vec if it grew large during a burst
            if batch.capacity() > 256 {
                batch = Vec::with_capacity(64);
            }
            continue;
        }

        // Phase 2: Check bulk
        if bulk_alive {
            if let Ok(buf) = bulk_rx.try_recv() {
                let bytes: &[u8] = &buf;
                let len = bytes.len() as u64;
                if let Err(e) = stream.write_all(bytes) {
                    report_error(error_tx, e);
                    return;
                }
                counters.record_sent(1, len);
                continue;
            }
        }

        // Phase 3: All channels empty — block on Select.
        // Issue 2: crossbeam Select is random. After select wakes, always
        // re-check control_rx first before processing the selected channel.
        // This ensures control priority even when Select picks a lower channel.
        if let Ok(frame) = control_rx.try_recv() {
            batch.clear();
            total_bytes = 0;
            batch.push(frame);
            total_bytes += batch[0].len();
            coalesce_all(control_rx, audit_rx, handoff_rx, data_rx,
                         audit_alive, handoff_alive, data_alive,
                         &mut batch, &mut total_bytes);
            let count = batch.len() as u64;
            if let Err(e) = write_batch(stream, &batch) {
                report_error(error_tx, e);
                return;
            }
            counters.record_sent(count, total_bytes as u64);
            continue;
        }

        let mut sel = crossbeam::channel::Select::new();
        let i_ctrl = sel.recv(control_rx);
        let i_audit = if audit_alive { Some(sel.recv(audit_rx)) } else { None };
        let i_handoff = if handoff_alive { Some(sel.recv(handoff_rx)) } else { None };
        let i_data = if data_alive { Some(sel.recv(data_rx)) } else { None };
        let i_bulk = if bulk_alive { Some(sel.recv(bulk_rx)) } else { None };

        let oper = sel.select();
        let idx = oper.index();

        // After Select wakes (random), always check control first.
        // Complete the selected operation to avoid partial-state, then
        // process control frames before lower-priority frames.
        if idx == i_ctrl {
            match oper.recv(control_rx) {
                Ok(frame) => {
                    batch.clear();
                    total_bytes = 0;
                    batch.push(frame);
                    total_bytes += batch[0].len();
                    coalesce_all(control_rx, audit_rx, handoff_rx, data_rx,
                                 audit_alive, handoff_alive, data_alive,
                                 &mut batch, &mut total_bytes);
                    let count = batch.len() as u64;
                    if let Err(e) = write_batch(stream, &batch) {
                        report_error(error_tx, e);
                        return;
                    }
                    counters.record_sent(count, total_bytes as u64);
                }
                Err(_) => {
                    tracing::debug!("write task: control_rx disconnected — exiting");
                    return;
                }
            }
        } else if i_bulk == Some(idx) {
            match oper.recv(bulk_rx) {
                Ok(buf) => {
                    // Before writing bulk, drain any control frames that arrived
                    batch.clear();
                    total_bytes = 0;
                    drain_small(control_rx, &mut batch, &mut total_bytes);
                    if !batch.is_empty() {
                        let count = batch.len() as u64;
                        if let Err(e) = write_batch(stream, &batch) {
                            report_error(error_tx, e);
                            return;
                        }
                        counters.record_sent(count, total_bytes as u64);
                    }
                    let bytes: &[u8] = &buf;
                    let len = bytes.len() as u64;
                    if let Err(e) = stream.write_all(bytes) {
                        report_error(error_tx, e);
                        return;
                    }
                    counters.record_sent(1, len);
                }
                Err(_) => { bulk_alive = false; }
            }
        } else {
            // Non-control, non-bulk channel woke. Complete the recv,
            // then coalesce with control-first priority.
            let woke_frame = if i_audit == Some(idx) {
                match oper.recv(audit_rx) {
                    Ok(f) => Some(f),
                    Err(_) => { audit_alive = false; None }
                }
            } else if i_handoff == Some(idx) {
                match oper.recv(handoff_rx) {
                    Ok(f) => Some(f),
                    Err(_) => { handoff_alive = false; None }
                }
            } else if i_data == Some(idx) {
                match oper.recv(data_rx) {
                    Ok(f) => Some(f),
                    Err(_) => { data_alive = false; None }
                }
            } else {
                None
            };

            if let Some(frame) = woke_frame {
                batch.clear();
                total_bytes = 0;
                // Control first, then the woke frame, then remaining channels
                drain_small(control_rx, &mut batch, &mut total_bytes);
                total_bytes += frame.len();
                batch.push(frame);
                if audit_alive { drain_small(audit_rx, &mut batch, &mut total_bytes); }
                if handoff_alive { drain_small(handoff_rx, &mut batch, &mut total_bytes); }
                if data_alive { drain_small(data_rx, &mut batch, &mut total_bytes); }
                let count = batch.len() as u64;
                if let Err(e) = write_batch(stream, &batch) {
                    report_error(error_tx, e);
                    return;
                }
                counters.record_sent(count, total_bytes as u64);
            }
        }
    }
}
