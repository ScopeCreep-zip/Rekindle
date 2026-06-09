//! Synchronization primitive irreducible floors: atomic ordering,
//! contended atomics, futex syscall, eventfd, spin-wait, RwLock
//! contention, and channel handoff.
//!
//! Report structure (criterion BenchmarkId decomposition):
//! - atomic_ordering:     violin — relaxed / release / seqcst / fence_seqcst
//! - contention_scaling:  line   — function=atomic, param=threads (linear)
//! - wakeup_comparison:   violin — eventfd / futex / spinwait
//! - rwlock_contention:   line   — function=read_only/mixed, param=threads (linear)
//! - channel_handoff:     violin — crossbeam_64 / crossbeam_256 / tokio_mpsc / rendezvous
//!
//! SamplingMode::Flat for wakeup and contended benches (thread lifecycle
//! inside bench_function). Auto for tight uncontended loops (atomic).
//!
//! Channel benches use the inout pattern (send + recv on same thread)
//! from crossbeam's own benchmarks to measure true handoff latency,
//! not just producer push latency.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU32, AtomicBool, Ordering};

use criterion::{BenchmarkId, Criterion, SamplingMode, Throughput};
use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession, bench_contended};

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    atomic_uncontended(c, session);
    contended_atomic(c, session);
    wakeup(c, session);
    rwlock(c, session);
    channel(c, session);
}

// ── Atomic ordering (uncontended) ───────────────────────────────────

fn atomic_uncontended(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("atomic_ordering");
    group.throughput(Throughput::Elements(1));

    for &(variant, ordering) in &[
        ("relaxed", Ordering::Relaxed),
        ("release", Ordering::Release),
        ("seqcst", Ordering::SeqCst),
    ] {
        let osc = format!("osc:lat/sync.atomic?contention=none&variant={variant}");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sync.atomic {variant}: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let counter = AtomicU64::new(0);
        group.bench_function(BenchmarkId::new(variant, "fetch_add"), |b| {
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters { counter.fetch_add(1, ordering); }
                start.elapsed()
            })
        });
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("atomic_ordering", variant, "fetch_add")));
    }

    // fence(SeqCst) variant
    {
        let id = OscId::parse(
            "osc:lat/sync.atomic?contention=none&variant=fence_seqcst",
            Profile::L1Conditions,
        ).expect("malformed OSC: sync.atomic fence_seqcst");
        if session.check_deps(&id).is_ok() {
            let counter = AtomicU64::new(0);
            group.bench_function(BenchmarkId::new("fence_seqcst", "fetch_add"), |b| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        counter.fetch_add(1, Ordering::Relaxed);
                        std::sync::atomic::fence(Ordering::SeqCst);
                    }
                    start.elapsed()
                })
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("atomic_ordering", "fence_seqcst", "fetch_add")));
        }
    }

    group.finish();
}

// ── Contended atomic ────────────────────────────────────────────────

fn contended_atomic(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("contention_scaling");
    group.throughput(Throughput::Elements(1));
    group.sampling_mode(SamplingMode::Flat);

    for &threads in &[2usize, 4, 8] {
        let osc = format!("osc:lat/sync.atomic?contention={threads}t");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sync.atomic contention={threads}t: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let counter = Arc::new(AtomicU64::new(0));
        let c_ref = Arc::clone(&counter);

        let fn_name = format!("atomic/{threads}");
        bench_contended(&mut group, &fn_name, threads, move |n| {
            for _ in 0..n { c_ref.fetch_add(1, Ordering::SeqCst); }
        });

        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("contention_scaling", &fn_name, "")));
    }

    group.finish();
}

// ── Wakeup floor (eventfd, futex, spin-wait) ────────────────────────

fn wakeup(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("wakeup_comparison");
    group.throughput(Throughput::Elements(1));
    group.sampling_mode(SamplingMode::Flat);

    // eventfd round-trip
    {
        let id = OscId::parse("osc:lat/sync.eventfd?contention=none", Profile::L1Conditions)
            .expect("malformed OSC: sync.eventfd");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("eventfd", "roundtrip"), |b| {
                let efd_to_echo = unsafe { libc::eventfd(0, libc::EFD_SEMAPHORE) };
                let efd_to_main = unsafe { libc::eventfd(0, libc::EFD_SEMAPHORE) };
                assert!(efd_to_echo >= 0 && efd_to_main >= 0);
                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();

                let echo = std::thread::spawn(move || {
                    let mut buf = [0u8; 8];
                    while r.load(Ordering::Acquire) {
                        let n = unsafe { libc::read(efd_to_echo, buf.as_mut_ptr() as *mut libc::c_void, 8) };
                        if n <= 0 { break; }
                        if !r.load(Ordering::Acquire) { break; }
                        let val: u64 = 1;
                        let n = unsafe { libc::write(efd_to_main, &val as *const u64 as *const libc::c_void, 8) };
                        if n <= 0 { break; }
                    }
                });

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let val: u64 = 1;
                        unsafe { libc::write(efd_to_echo, &val as *const u64 as *const libc::c_void, 8); }
                        let mut buf = [0u8; 8];
                        unsafe { libc::read(efd_to_main, buf.as_mut_ptr() as *mut libc::c_void, 8); }
                    }
                    start.elapsed()
                });

                running.store(false, Ordering::Release);
                let val: u64 = 1;
                unsafe { libc::write(efd_to_echo, &val as *const u64 as *const libc::c_void, 8); }
                let _ = echo.join();
                unsafe { libc::close(efd_to_echo); libc::close(efd_to_main); }
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("wakeup_comparison", "eventfd", "roundtrip")));
        }
    }

    // futex syscall round-trip
    {
        let id = OscId::parse("osc:lat/sync.futex?contention=none", Profile::L1Conditions)
            .expect("malformed OSC: sync.futex");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("futex", "roundtrip"), |b| {
                let flag_a = Arc::new(AtomicU32::new(0));
                let flag_b = Arc::new(AtomicU32::new(0));
                let fa = flag_a.clone();
                let fb = flag_b.clone();
                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();

                let echo = std::thread::spawn(move || {
                    while r.load(Ordering::Acquire) {
                        while fa.load(Ordering::Acquire) == 0 {
                            if !r.load(Ordering::Relaxed) { return; }
                            unsafe {
                                libc::syscall(libc::SYS_futex,
                                    fa.as_ptr(), libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                                    0u32, std::ptr::null::<libc::timespec>());
                            }
                        }
                        fa.store(0, Ordering::Release);
                        fb.store(1, Ordering::Release);
                        unsafe {
                            libc::syscall(libc::SYS_futex,
                                fb.as_ptr(), libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG, 1i32);
                        }
                    }
                });

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        flag_a.store(1, Ordering::Release);
                        unsafe {
                            libc::syscall(libc::SYS_futex,
                                flag_a.as_ptr(), libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG, 1i32);
                        }
                        while flag_b.load(Ordering::Acquire) == 0 {
                            unsafe {
                                libc::syscall(libc::SYS_futex,
                                    flag_b.as_ptr(), libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                                    0u32, std::ptr::null::<libc::timespec>());
                            }
                        }
                        flag_b.store(0, Ordering::Release);
                    }
                    start.elapsed()
                });

                running.store(false, Ordering::Release);
                flag_a.store(1, Ordering::Release);
                unsafe {
                    libc::syscall(libc::SYS_futex,
                        flag_a.as_ptr(), libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG, 1i32);
                }
                let _ = echo.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("wakeup_comparison", "futex", "roundtrip")));
        }
    }

    // Spin-wait round-trip
    {
        let id = OscId::parse("osc:lat/sync.spinwait?contention=none", Profile::L1Conditions)
            .expect("malformed OSC: sync.spinwait");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("spinwait", "roundtrip"), |b| {
                let flag_a = Arc::new(AtomicU64::new(0));
                let flag_b = Arc::new(AtomicU64::new(0));
                let fa = flag_a.clone();
                let fb = flag_b.clone();
                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();

                let echo = std::thread::spawn(move || {
                    loop {
                        while fa.load(Ordering::Acquire) == 0 {
                            std::hint::spin_loop();
                        }
                        if !r.load(Ordering::Acquire) { return; }
                        fa.store(0, Ordering::Release);
                        fb.store(1, Ordering::Release);
                    }
                });

                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        flag_a.store(1, Ordering::Release);
                        while flag_b.load(Ordering::Acquire) == 0 { std::hint::spin_loop(); }
                        flag_b.store(0, Ordering::Release);
                    }
                    start.elapsed()
                });

                running.store(false, Ordering::Release);
                flag_a.store(1, Ordering::Release);
                let _ = echo.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("wakeup_comparison", "spinwait", "roundtrip")));
        }
    }

    // Record sync.wake as established — satisfies io.rtt's dependency
    {
        let wake_id = OscId::parse("osc:lat/sync.wake", Profile::L0Core).unwrap();
        session.record(&wake_id, 0.0, 0.0, 0.0, 0, "derived.from.wakeup_floor");
    }

    group.finish();
}

// ── RwLock contention ───────────────────────────────────────────────

struct RwLockPayload {
    envelope_key: [u8; 32],
    header_key: [u8; 32],
    cipher: Arc<AtomicU64>,
}

fn rwlock(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("rwlock_contention");
    group.throughput(Throughput::Elements(1));
    group.sampling_mode(SamplingMode::Flat);

    let lock = Arc::new(parking_lot::RwLock::new(RwLockPayload {
        envelope_key: [0x11; 32],
        header_key: [0x22; 32],
        cipher: Arc::new(AtomicU64::new(0)),
    }));

    // Pure read contention
    for &readers in &[1usize, 2, 4, 8] {
        let osc = format!("osc:lat/sync.rwlock?contention={readers}t&variant=read_only");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sync.rwlock read {readers}t: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let l = Arc::clone(&lock);
        let fn_name = format!("read_only/{readers}");
        bench_contended(&mut group, &fn_name, readers, move |n| {
            for _ in 0..n {
                let guard = l.read();
                std::hint::black_box(&guard.envelope_key);
                std::hint::black_box(&guard.header_key);
                let _c = Arc::clone(&guard.cipher);
                std::hint::black_box(&_c);
                drop(guard);
            }
        });

        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("rwlock_contention", &fn_name, "")));
    }

    // Mixed: N readers + 1 writer
    for &readers in &[2usize, 4, 8] {
        let osc = format!("osc:lat/sync.rwlock?contention={readers}t+1w&variant=mixed");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sync.rwlock mixed {readers}t: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let l_write = Arc::clone(&lock);
        let l_read = Arc::clone(&lock);
        let readers_str = format!("{readers}");

        group.bench_function(BenchmarkId::new("mixed", &readers_str), |b| {
            let writer_running = Arc::new(AtomicBool::new(true));
            let wr = writer_running.clone();
            let wl = Arc::clone(&l_write);

            let writer = std::thread::spawn(move || {
                let mut gen = 0u8;
                while wr.load(Ordering::Acquire) {
                    {
                        let mut g = wl.write();
                        gen = gen.wrapping_add(1);
                        g.envelope_key = [gen; 32];
                        g.header_key = [gen; 32];
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });

            let go = Arc::new(std::sync::Barrier::new(readers + 1));
            let done = Arc::new(std::sync::Barrier::new(readers + 1));
            let iterations = Arc::new(AtomicU64::new(0));
            let running = Arc::new(AtomicBool::new(true));

            let handles: Vec<_> = (0..readers).map(|_| {
                let l = Arc::clone(&l_read);
                let g = Arc::clone(&go);
                let d = Arc::clone(&done);
                let it = Arc::clone(&iterations);
                let r = Arc::clone(&running);
                std::thread::spawn(move || {
                    loop {
                        g.wait();
                        if !r.load(Ordering::Acquire) { d.wait(); break; }
                        let n = it.load(Ordering::Acquire);
                        for _ in 0..n {
                            let guard = l.read();
                            std::hint::black_box(&guard.envelope_key);
                            std::hint::black_box(&guard.header_key);
                            let _c = Arc::clone(&guard.cipher);
                            std::hint::black_box(&_c);
                            drop(guard);
                        }
                        d.wait();
                    }
                })
            }).collect();

            b.iter_custom(|iters| {
                let per_thread = iters.div_ceil(readers as u64);
                iterations.store(per_thread, Ordering::Release);
                go.wait();
                let start = std::time::Instant::now();
                done.wait();
                let elapsed = start.elapsed();
                let actual = per_thread * readers as u64;
                elapsed.mul_f64(iters as f64 / actual as f64)
            });

            running.store(false, Ordering::Release);
            iterations.store(0, Ordering::Release);
            go.wait();
            done.wait();
            for h in handles { let _ = h.join(); }
            writer_running.store(false, Ordering::Release);
            let _ = writer.join();
        });

        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("rwlock_contention", "mixed", &readers_str)));
    }

    group.finish();
}

// ── Channel handoff ─────────────────────────────────────────────────

fn channel(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("channel_handoff");
    group.throughput(Throughput::Elements(1));
    group.sampling_mode(SamplingMode::Flat);

    // Crossbeam bounded — inout pattern
    for &cap in &[64, 256] {
        let osc = format!("osc:lat/sync.channel?variant=crossbeam_cap{cap}&size=64&alloc=none&contention=none");
        let id = OscId::parse(&osc, Profile::L1Conditions)
            .unwrap_or_else(|e| panic!("malformed OSC: sync.channel cap={cap}: {e}"));

        if let Err(skip) = session.check_deps(&id) {
            tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
            continue;
        }

        let fn_name = format!("crossbeam_{cap}");
        group.bench_function(BenchmarkId::new(&fn_name, "64B"), |b| {
            let (tx, rx) = crossbeam::channel::bounded::<[u8; 64]>(cap);
            let msg = [0x42u8; 64];
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    tx.send(msg).unwrap();
                    std::hint::black_box(rx.recv().unwrap());
                }
                start.elapsed()
            });
        });
        session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
            Some(("channel_handoff", &fn_name, "64B")));
    }

    // tokio mpsc
    {
        let id = OscId::parse(
            "osc:lat/sync.channel?variant=tokio_mpsc&size=64&alloc=none&contention=none",
            Profile::L1Conditions,
        ).expect("malformed OSC: sync.channel tokio_mpsc");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("tokio_mpsc", "64B"), |b| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2).enable_all().build().unwrap();
                let (tx, mut rx) = tokio::sync::mpsc::channel::<[u8; 64]>(256);
                let (echo_tx, mut echo_rx) = tokio::sync::mpsc::channel::<[u8; 64]>(256);
                rt.spawn(async move {
                    while let Some(msg) = rx.recv().await {
                        if echo_tx.send(msg).await.is_err() { break; }
                    }
                });
                let msg = [0x42u8; 64];
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        tx.blocking_send(msg).unwrap();
                        std::hint::black_box(echo_rx.blocking_recv().unwrap());
                    }
                    start.elapsed()
                });
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("channel_handoff", "tokio_mpsc", "64B")));
        }
    }

    // Rendezvous
    {
        let id = OscId::parse(
            "osc:lat/sync.channel?variant=crossbeam_rendezvous&size=0&alloc=none&contention=none",
            Profile::L1Conditions,
        ).expect("malformed OSC: sync.channel rendezvous");
        if session.check_deps(&id).is_ok() {
            group.bench_function(BenchmarkId::new("rendezvous", "0B"), |b| {
                let (tx, rx) = crossbeam::channel::bounded::<()>(0);
                let (echo_tx, echo_rx) = crossbeam::channel::bounded::<()>(0);
                let drain = std::thread::spawn(move || {
                    while let Ok(()) = rx.recv() {
                        if echo_tx.send(()).is_err() { break; }
                    }
                });
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        tx.send(()).unwrap();
                        echo_rx.recv().unwrap();
                    }
                    start.elapsed()
                });
                drop(tx);
                let _ = drain.join();
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("channel_handoff", "rendezvous", "0B")));
        }
    }

    group.finish();
}
