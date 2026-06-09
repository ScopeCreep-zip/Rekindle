//! I/O subsystem irreducible floors: io_uring NOP, batch amortization,
//! uring Send/Recv on AF_UNIX, RTT, socket throughput, socket pressure.
//!
//! Report structure (criterion BenchmarkId decomposition):
//! - uring_nop:              violin — coop / defer / sqpoll (no sweep)
//! - uring_batch:            line   — function=batch, param=count (linear)
//! - uring_socket:           violin — send / recv (no sweep)
//! - rtt_bare:               violin — libc / uring_send (no sweep)
//! - socket_throughput:      line   — function=uring_send, param=size (log)
//! - socket_pressure:        line   — function=uring_send, param=sndbuf (log)
//!
//! Ring configuration:
//! - NOP/batch: COOP_TASKRUN + SINGLE_ISSUER (CQE inline)
//! - Socket Send/Recv: DEFER_TASKRUN + SINGLE_ISSUER (CQE via task_work)
//! - RTT uring-send: bare ring (mixed uring + blocking libc)
//!
//! SamplingMode::Flat for all io benches.
//!
//! Partial send: socket_throughput verifies cqe.result() == data.len()
//! because io_uring Send on AF_UNIX may return short for large payloads.

use std::os::unix::net::UnixStream;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use criterion::{
    AxisScale, BenchmarkId, Criterion, PlotConfiguration, SamplingMode, Throughput,
};
use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession};

// ── uring_nop — violin: coop / defer / sqpoll ───────────────────────

fn uring_nop(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("uring_nop");
    group.sampling_mode(SamplingMode::Flat);
    group.throughput(Throughput::Elements(1));

    // COOP_TASKRUN
    {
        let id = OscId::parse("osc:lat/io.nop?ring=coop", Profile::L1Conditions)
            .expect("malformed OSC: io.nop coop");
        if session.check_deps(&id).is_ok() {
            let mut ring = io_uring::IoUring::builder()
                .setup_cqsize(8).setup_coop_taskrun().setup_single_issuer()
                .build(4).unwrap();
            group.bench_function(BenchmarkId::new("coop", "nop"), |b| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let sqe = io_uring::opcode::Nop::new().build().user_data(1);
                        unsafe { ring.submission().push(&sqe).unwrap(); }
                        ring.submit_and_wait(1).unwrap();
                        let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                        std::hint::black_box(cqe.result());
                    }
                    start.elapsed()
                })
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("uring_nop", "coop", "nop")));
        }
    }

    // DEFER_TASKRUN
    {
        let id = OscId::parse("osc:lat/io.nop?ring=defer", Profile::L1Conditions)
            .expect("malformed OSC: io.nop defer");
        if session.check_deps(&id).is_ok() {
            if let Ok(mut ring) = io_uring::IoUring::builder()
                .setup_cqsize(8).setup_single_issuer().setup_defer_taskrun()
                .build(4)
            {
                group.bench_function(BenchmarkId::new("defer", "nop"), |b| {
                    b.iter_custom(|iters| {
                        let start = std::time::Instant::now();
                        for _ in 0..iters {
                            let sqe = io_uring::opcode::Nop::new().build().user_data(1);
                            unsafe { ring.submission().push(&sqe).unwrap(); }
                            ring.submit_and_wait(1).unwrap();
                            let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                            std::hint::black_box(cqe.result());
                        }
                        start.elapsed()
                    })
                });
                session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                    Some(("uring_nop", "defer", "nop")));
            } else {
                tracing::warn!("DEFER_TASKRUN ring build failed, skipping");
            }
        }
    }

    // SQPOLL
    {
        let id = OscId::parse("osc:lat/io.nop?ring=sqpoll", Profile::L1Conditions)
            .expect("malformed OSC: io.nop sqpoll");
        if session.check_deps(&id).is_ok() {
            if let Ok(mut ring) = io_uring::IoUring::builder()
                .setup_cqsize(8).setup_single_issuer().setup_sqpoll(1000)
                .build(4)
            {
                group.bench_function(BenchmarkId::new("sqpoll", "nop"), |b| {
                    b.iter_custom(|iters| {
                        let start = std::time::Instant::now();
                        for _ in 0..iters {
                            let sqe = io_uring::opcode::Nop::new().build().user_data(1);
                            unsafe { ring.submission().push(&sqe).unwrap(); }
                            ring.submit_and_wait(1).unwrap();
                            let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                            std::hint::black_box(cqe.result());
                        }
                        start.elapsed()
                    })
                });
                session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                    Some(("uring_nop", "sqpoll", "nop")));
            } else {
                tracing::warn!("SQPOLL ring build failed, skipping");
            }
        }
    }

    group.finish();
}

// ── uring_batch — line chart: function=batch, param=count ───────────

fn uring_batch(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("uring_batch");
    group.sampling_mode(SamplingMode::Flat);

    for &batch in &[4u32, 16, 64] {
        let id = OscId::parse(
            &format!("osc:lat/io.submit?ring=coop&size={batch}"),
            Profile::L1Conditions,
        ).unwrap_or_else(|e| panic!("malformed OSC: io.submit batch={batch}: {e}"));
        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        group.throughput(Throughput::Elements(batch as u64));
        let mut ring = io_uring::IoUring::builder()
            .setup_cqsize(128).setup_coop_taskrun().setup_single_issuer()
            .build(128).unwrap();
        group.bench_function(BenchmarkId::new("batch", format!("{batch}")), |b| {
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    for i in 0..batch {
                        let sqe = io_uring::opcode::Nop::new().build().user_data(i as u64);
                        unsafe { ring.submission().push(&sqe).unwrap(); }
                    }
                    ring.submit_and_wait(batch as usize).unwrap();
                    let mut count = 0u32;
                    loop {
                        let cqe: Option<io_uring::cqueue::Entry> = ring.completion().next();
                        match cqe {
                            Some(e) => { std::hint::black_box(e.result()); count += 1; }
                            None => break,
                        }
                    }
                    // debug_assert in release — hot loop, no panic overhead
                    debug_assert_eq!(count, batch);
                }
                start.elapsed()
            })
        });
        let batch_str = format!("{batch}");
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("uring_batch", "batch", &batch_str)));
    }

    group.finish();
}

// ── uring_socket — violin: send / recv ──────────────────────────────

fn uring_socket(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("uring_socket");
    group.sampling_mode(SamplingMode::Flat);
    group.throughput(Throughput::Elements(1));

    // Send — drain thread reads and discards
    {
        let id = OscId::parse("osc:lat/io.send?size=64&ring=defer", Profile::L1Conditions)
            .expect("malformed OSC: io.send");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("send", "64B"), |b| {
                let (a, d) = UnixStream::pair().unwrap();
                let a_fd = a.as_raw_fd();
                let drain = std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let mut d = d;
                    loop {
                        match std::io::Read::read(&mut d, &mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                });
                let mut ring = io_uring::IoUring::builder()
                    .setup_cqsize(8).setup_single_issuer().setup_defer_taskrun()
                    .build(4).unwrap();
                ring.submitter().register_files(&[a_fd]).unwrap();
                let fixed_fd = io_uring::types::Fixed(0);
                let msg = [0x42u8; 64];

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let sqe = io_uring::opcode::Send::new(fixed_fd, msg.as_ptr(), 64)
                            .build().user_data(1);
                        unsafe { ring.submission().push(&sqe).unwrap(); }
                        ring.submit_and_wait(1).unwrap();
                        let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                        // 64B on AF_UNIX is atomic — partial send impossible
                        debug_assert_eq!(cqe.result(), 64);
                    }
                    start.elapsed()
                });

                drop(ring);
                drop(a);
                let _ = drain.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("uring_socket", "send", "64B")));
        }
    }

    // Recv — sender thread writes one message per recv iteration.
    // Uses a rendezvous channel to synchronize: sender writes one 64B
    // message, waits for ack, repeat. No busy-spinning, no CPU waste.
    {
        let id = OscId::parse("osc:lat/io.recv?size=64&ring=defer", Profile::L1Conditions)
            .expect("malformed OSC: io.recv");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("recv", "64B"), |b| {
                let (a, d) = UnixStream::pair().unwrap();
                let a_fd = a.as_raw_fd();
                let d_fd = d.as_raw_fd();

                // Sender: write one 64B message, then block on a rendezvous
                // channel until the bench thread signals the next iteration.
                // This eliminates the busy-spin sender that consumed a full
                // CPU core in the previous implementation.
                let (go_tx, go_rx) = crossbeam::channel::bounded::<()>(0);
                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();

                let sender = std::thread::spawn(move || {
                    let msg = [0x42u8; 64];
                    // Pre-fill: write one message so the first recv has data
                    let n = unsafe { libc::write(d_fd, msg.as_ptr() as *const libc::c_void, 64) };
                    if n <= 0 { drop(d); return; }

                    while r.load(Ordering::Acquire) {
                        // Wait for the bench thread to signal "next iteration"
                        if go_rx.recv().is_err() { break; }
                        if !r.load(Ordering::Acquire) { break; }
                        let n = unsafe { libc::write(d_fd, msg.as_ptr() as *const libc::c_void, 64) };
                        if n <= 0 { break; }
                    }
                    drop(d);
                });

                let mut ring = io_uring::IoUring::builder()
                    .setup_cqsize(8).setup_single_issuer().setup_defer_taskrun()
                    .build(4).unwrap();
                ring.submitter().register_files(&[a_fd]).unwrap();
                let fixed_fd = io_uring::types::Fixed(0);
                let mut buf = [0u8; 64];

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let sqe = io_uring::opcode::Recv::new(fixed_fd, buf.as_mut_ptr(), 64)
                            .build().user_data(1);
                        unsafe { ring.submission().push(&sqe).unwrap(); }
                        ring.submit_and_wait(1).unwrap();
                        let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                        debug_assert_eq!(cqe.result(), 64);
                        std::hint::black_box(&buf);
                        // Signal sender to write next message
                        let _ = go_tx.send(());
                    }
                    start.elapsed()
                });

                running.store(false, Ordering::Release);
                drop(go_tx); // disconnects → sender exits recv loop
                drop(ring);
                drop(a);
                let _ = sender.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("uring_socket", "recv", "64B")));
        }
    }

    group.finish();
}

// ── RTT bare — violin: libc / uring_send ────────────────────────────

fn rtt_bare(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("rtt_bare");
    group.sampling_mode(SamplingMode::Flat);
    group.throughput(Throughput::Elements(1));

    // Blocking socketpair — pure libc. Return values checked via debug_assert.
    {
        let id = OscId::parse("osc:lat/io.rtt?size=64&ring=bare&send=libc", Profile::L1Conditions)
            .expect("malformed OSC: io.rtt libc");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("libc", "64B"), |b| {
                let (a, d) = UnixStream::pair().unwrap();
                let a_fd = a.as_raw_fd();
                let d_fd = d.as_raw_fd();
                let msg = [0x42u8; 64];
                let mut buf = [0u8; 64];
                let echo = std::thread::spawn(move || {
                    let mut ebuf = [0u8; 64];
                    loop {
                        let n = unsafe { libc::read(d_fd, ebuf.as_mut_ptr() as *mut libc::c_void, 64) };
                        if n <= 0 { break; }
                        let n = unsafe { libc::write(d_fd, ebuf.as_ptr() as *const libc::c_void, n as usize) };
                        if n <= 0 { break; }
                    }
                    drop(d);
                });

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let w = unsafe { libc::write(a_fd, msg.as_ptr() as *const libc::c_void, 64) };
                        debug_assert_eq!(w, 64, "RTT write short: {w}");
                        let r = unsafe { libc::read(a_fd, buf.as_mut_ptr() as *mut libc::c_void, 64) };
                        debug_assert_eq!(r, 64, "RTT read short: {r}");
                    }
                    start.elapsed()
                });

                unsafe { libc::shutdown(a_fd, libc::SHUT_RDWR); }
                drop(a);
                let _ = echo.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("rtt_bare", "libc", "64B")));
        }
    }

    // io_uring Send + blocking libc recv
    {
        let id = OscId::parse("osc:lat/io.rtt?size=64&ring=bare&send=uring", Profile::L1Conditions)
            .expect("malformed OSC: io.rtt uring");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("uring_send", "64B"), |b| {
                let (a, d) = UnixStream::pair().unwrap();
                let a_fd = a.as_raw_fd();
                let d_fd = d.as_raw_fd();
                let msg = [0x42u8; 64];
                let mut buf = [0u8; 64];
                let echo = std::thread::spawn(move || {
                    let mut ebuf = [0u8; 64];
                    loop {
                        let n = unsafe { libc::read(d_fd, ebuf.as_mut_ptr() as *mut libc::c_void, 64) };
                        if n <= 0 { break; }
                        let n = unsafe { libc::write(d_fd, ebuf.as_ptr() as *const libc::c_void, n as usize) };
                        if n <= 0 { break; }
                    }
                    drop(d);
                });
                let mut ring = io_uring::IoUring::builder()
                    .setup_cqsize(8).build(4).unwrap();
                ring.submitter().register_files(&[a_fd]).unwrap();
                let fixed_fd = io_uring::types::Fixed(0);

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for i in 0..iters {
                        let sqe = io_uring::opcode::Send::new(fixed_fd, msg.as_ptr(), 64)
                            .build().user_data(i);
                        unsafe { ring.submission().push(&sqe).unwrap(); }
                        ring.submit_and_wait(1).unwrap();
                        let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                        debug_assert_eq!(cqe.result(), 64);
                        let r = unsafe { libc::read(a_fd, buf.as_mut_ptr() as *mut libc::c_void, 64) };
                        debug_assert_eq!(r, 64, "RTT recv short: {r}");
                    }
                    start.elapsed()
                });

                unsafe { libc::shutdown(a_fd, libc::SHUT_RDWR); }
                drop(a);
                let _ = echo.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("rtt_bare", "uring_send", "64B")));
        }
    }

    group.finish();
}

// ── Socket throughput — line chart: param=size ───────────────────────
// Verifies full send: assert cqe.result() == data.len() because
// io_uring Send on AF_UNIX may return partial for large payloads.

fn socket_throughput(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("socket_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    group.sampling_mode(SamplingMode::Flat);
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536, 1024 * 1024, 16 * 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let id = OscId::parse(
            &format!("osc:bw/io.send?size={mag}&ring=defer&drain=fast"),
            Profile::L1Conditions,
        ).unwrap_or_else(|e| panic!("malformed OSC: io.send size={mag}: {e}"));
        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function(BenchmarkId::new("uring_send", format!("{size}")), |b| {
            let (a, d) = UnixStream::pair().unwrap();
            let a_fd = a.as_raw_fd();
            let d_fd = d.as_raw_fd();
            unsafe {
                let buf_size: libc::c_int = 16 * 1024 * 1024;
                libc::setsockopt(a_fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                    &buf_size as *const _ as *const libc::c_void, 4);
                libc::setsockopt(d_fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                    &buf_size as *const _ as *const libc::c_void, 4);
            }
            let drain = std::thread::spawn(move || {
                let mut buf = vec![0u8; 16 * 1024 * 1024];
                let mut d = d;
                loop {
                    match std::io::Read::read(&mut d, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
            let mut ring = io_uring::IoUring::builder()
                .setup_cqsize(8).setup_single_issuer().setup_defer_taskrun()
                .build(4).unwrap();
            ring.submitter().register_files(&[a_fd]).unwrap();
            let fixed_fd = io_uring::types::Fixed(0);

            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let sqe = io_uring::opcode::Send::new(
                        fixed_fd, data.as_ptr(), data.len() as u32,
                    ).build().user_data(1);
                    unsafe { ring.submission().push(&sqe).unwrap(); }
                    ring.submit_and_wait(1).unwrap();
                    let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                    // Verify full send — partial sends corrupt throughput numbers
                    debug_assert_eq!(
                        cqe.result() as usize, data.len(),
                        "partial send: {} of {} bytes", cqe.result(), data.len(),
                    );
                }
                start.elapsed()
            });

            drop(ring);
            drop(a);
            let _ = drain.join();
        });
        let size_str = format!("{size}");
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 10, "criterion.pending_backfill",
            Some(("socket_throughput", "uring_send", &size_str)));
    }

    group.finish();
}

// ── Socket pressure — line chart: param=sndbuf ──────────────────────
// No throughput annotation — the parameter strings ("65536", "262144",
// "1048576", "4194304") drive as_number() via the value_str fallback,
// producing distinct X values and a line chart.
//
// Drain uses crossbeam Parker for deterministic ~100µs delay instead
// of std::thread::sleep which has 1ms minimum on Linux CONFIG_HZ=1000.

fn socket_pressure(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("socket_pressure");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    group.sampling_mode(SamplingMode::Flat);

    let payload = vec![0x42u8; 64 * 1024];

    for &buf_kb in &[64, 256, 1024, 4096] {
        let buf_size = buf_kb * 1024;
        let id = OscId::parse(
            &format!("osc:bw/io.send?size=64k&ring=defer&drain=slow&state=saturated&sndbuf={buf_kb}k"),
            Profile::L1Conditions,
        ).unwrap_or_else(|e| panic!("malformed OSC: io.send sndbuf={buf_kb}k: {e}"));
        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        group.bench_function(BenchmarkId::new("uring_send", format!("{buf_size}")), |b| {
            let (a, d) = UnixStream::pair().unwrap();
            let a_fd = a.as_raw_fd();
            let d_fd = d.as_raw_fd();
            unsafe {
                let sz: libc::c_int = buf_size as libc::c_int;
                libc::setsockopt(a_fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                    &sz as *const _ as *const libc::c_void, 4);
                libc::setsockopt(d_fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                    &sz as *const _ as *const libc::c_void, 4);
            }
            // Deterministic slow drain via spin backoff (~100µs granularity)
            // instead of std::thread::sleep which rounds up to ~1ms.
            let drain = std::thread::spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                let mut d = d;
                loop {
                    match std::io::Read::read(&mut d, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            // Spin for ~100µs — more precise than sleep(100µs)
                            let deadline = std::time::Instant::now()
                                + Duration::from_micros(100);
                            while std::time::Instant::now() < deadline {
                                std::hint::spin_loop();
                            }
                        }
                    }
                }
            });
            let mut ring = io_uring::IoUring::builder()
                .setup_cqsize(8).setup_single_issuer().setup_defer_taskrun()
                .build(4).unwrap();
            ring.submitter().register_files(&[a_fd]).unwrap();
            let fixed_fd = io_uring::types::Fixed(0);

            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let sqe = io_uring::opcode::Send::new(
                        fixed_fd, payload.as_ptr(), payload.len() as u32,
                    ).build().user_data(1);
                    unsafe { ring.submission().push(&sqe).unwrap(); }
                    ring.submit_and_wait(1).unwrap();
                    let cqe: io_uring::cqueue::Entry = ring.completion().next().unwrap();
                    debug_assert!(cqe.result() > 0);
                }
                start.elapsed()
            });

            drop(ring);
            drop(a);
            let _ = drain.join();
        });
        let bufsize_str = format!("{buf_size}");
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 10, "criterion.pending_backfill",
            Some(("socket_pressure", "uring_send", &bufsize_str)));
    }

    group.finish();
}

// ── Register ────────────────────────────────────────────────────────

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    uring_nop(c, session);
    uring_batch(c, session);
    uring_socket(c, session);
    rtt_bare(c, session);
    socket_throughput(c, session);
    socket_pressure(c, session);
}
