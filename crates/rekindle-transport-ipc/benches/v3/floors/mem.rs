//! Memory subsystem irreducible floors: DRAM bandwidth, cache hierarchy,
//! memcpy scaling, allocation lifecycle, page fault cost, blake3 scaling.
//!
//! Report structure (criterion BenchmarkId decomposition):
//! - dram_bandwidth:       violin — triad / seq_read / seq_write (no sweep)
//! - cache_hierarchy:      line   — function=L1/L2/L3/DRAM, param=size (log)
//! - memcpy_scaling:       line   — function=copy, param=size (log)
//! - allocation_lifecycle: line   — function=fresh/reuse/reuse_zeroed, param=size (log)
//! - page_fault:           line   — function=cold/populate/warm, param=size (log)
//! - blake3_scaling:       line   — function=blake3, param=size (log)
//!
//! SamplingMode:
//! - dram_bandwidth, page_fault: Flat (ms-scale, mmap/munmap not linearly scalable)
//! - cache_hierarchy, memcpy_scaling, allocation_lifecycle, blake3_scaling: Auto
//!   (tight loops where Linear regression gives slope = per-iteration cost)

use std::time::Duration;

use criterion::{
    AxisScale, Criterion, PlotConfiguration, SamplingMode, Throughput,
};
use rekindle_transport_ipc::calibrate::{CalibratedSession, calibrated_bench};

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    dram_bandwidth(c, session);
    cache_hierarchy(c, session);
    memcpy(c, session);
    allocation(c, session);
    page_fault(c, session);
    blake3(c, session);
}

// ── DRAM bandwidth ──────────────────────────────────────────────────
// Violin: triad / seq_read / seq_write — single-point, no parameter sweep.
// Parameter "64m" is intentionally non-numeric: value_str.parse::<f64>()
// fails → value_type() = None → no line chart. Correct for a violin-only
// group with no X-axis sweep.

fn dram_bandwidth(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("dram_bandwidth");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    group.sampling_mode(SamplingMode::Flat);

    const N: usize = 64 * 1024 * 1024 / 8;
    let mut a = vec![1.0f64; N];
    let b_src = vec![2.0f64; N];
    let cc = vec![3.0f64; N];
    let scalar = 0.42f64;

    calibrated_bench(session, &mut group, "dram_bandwidth",
        "osc:bw/mem.write?size=64m&cache=dram&order=sequential",
        Throughput::Bytes((N * 24) as u64),
        "triad", "64m", 10,
        |iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                for i in 0..N { a[i] = b_src[i] + scalar * cc[i]; }
                std::hint::black_box(&a);
            }
            start.elapsed()
        },
    );

    let src = vec![0x42u8; 64 * 1024 * 1024];
    calibrated_bench(session, &mut group, "dram_bandwidth",
        "osc:bw/mem.read?size=64m&cache=dram&order=sequential",
        Throughput::Bytes(64 * 1024 * 1024),
        "seq_read", "64m", 10,
        |iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let mut s0: u64 = 0; let mut s1: u64 = 0;
                let mut s2: u64 = 0; let mut s3: u64 = 0;
                let mut s4: u64 = 0; let mut s5: u64 = 0;
                let mut s6: u64 = 0; let mut s7: u64 = 0;
                for chunk in src.chunks_exact(64) {
                    s0 = s0.wrapping_add(u64::from_le_bytes(chunk[0..8].try_into().unwrap()));
                    s1 = s1.wrapping_add(u64::from_le_bytes(chunk[8..16].try_into().unwrap()));
                    s2 = s2.wrapping_add(u64::from_le_bytes(chunk[16..24].try_into().unwrap()));
                    s3 = s3.wrapping_add(u64::from_le_bytes(chunk[24..32].try_into().unwrap()));
                    s4 = s4.wrapping_add(u64::from_le_bytes(chunk[32..40].try_into().unwrap()));
                    s5 = s5.wrapping_add(u64::from_le_bytes(chunk[40..48].try_into().unwrap()));
                    s6 = s6.wrapping_add(u64::from_le_bytes(chunk[48..56].try_into().unwrap()));
                    s7 = s7.wrapping_add(u64::from_le_bytes(chunk[56..64].try_into().unwrap()));
                }
                std::hint::black_box(
                    s0.wrapping_add(s1).wrapping_add(s2).wrapping_add(s3)
                      .wrapping_add(s4).wrapping_add(s5).wrapping_add(s6).wrapping_add(s7),
                );
            }
            start.elapsed()
        },
    );

    let mut dst = vec![0u8; 64 * 1024 * 1024];
    calibrated_bench(session, &mut group, "dram_bandwidth",
        "osc:bw/mem.zero?size=64m&cache=dram",
        Throughput::Bytes(64 * 1024 * 1024),
        "seq_write", "64m", 10,
        |iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                dst.fill(0x42);
                std::hint::black_box(&dst);
            }
            start.elapsed()
        },
    );

    group.finish();
}

// ── Cache hierarchy ─────────────────────────────────────────────────
// Line chart: function=L1/L2/L3/DRAM, parameter=size (raw numeric).
// Shows bandwidth cliff between cache levels.
// SamplingMode::Auto → criterion chooses Linear for these tight copy loops,
// producing regression plots with slope = per-iteration cost.

fn cache_hierarchy(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("cache_hierarchy");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(5));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &(size, cache_label) in &[
        (8 * 1024, "L1"),     (16 * 1024, "L1"),
        (64 * 1024, "L2"),    (128 * 1024, "L2"),
        (1024 * 1024, "L3"),  (4 * 1024 * 1024, "L3"),
        (16 * 1024 * 1024, "DRAM"), (32 * 1024 * 1024, "DRAM"),
    ] {
        let mag = super::fmt_mag(size);
        let osc = format!("osc:bw/mem.copy?size={mag}&cache={}", cache_label.to_ascii_lowercase());
        let src = vec![0x42u8; size];
        let mut dst = vec![0u8; size];

        calibrated_bench(session, &mut group, "cache_hierarchy", &osc,
            Throughput::Bytes((2 * size) as u64),
            cache_label, &format!("{size}"), 20,
            |iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    dst.copy_from_slice(&src);
                    std::hint::black_box(&dst);
                }
                start.elapsed()
            },
        );
    }

    group.finish();
}

// ── memcpy scaling ──────────────────────────────────────────────────
// Line chart: function=copy, parameter=size. Shows memcpy throughput curve.

fn memcpy(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("memcpy_scaling");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536, 1024 * 1024, 16 * 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let osc = format!("osc:bw/mem.copy?size={mag}");
        let src: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut dst = vec![0u8; size];

        calibrated_bench(session, &mut group, "memcpy_scaling", &osc,
            Throughput::Bytes((2 * size) as u64),
            "copy", &format!("{size}"), 20,
            |iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    dst.copy_from_slice(&src);
                    std::hint::black_box(&dst);
                }
                start.elapsed()
            },
        );
    }

    group.finish();
}

// ── Allocation lifecycle ────────────────────────────────────────────
// Line chart: function=fresh/reuse/reuse_zeroed, parameter=size.
// Shows when zeroing dominates allocation cost.

fn allocation(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("allocation_lifecycle");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536, 1024 * 1024, 16 * 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let param = format!("{size}");

        // fresh alloc+drop
        calibrated_bench(session, &mut group, "allocation_lifecycle",
            &format!("osc:lat/mem.alloc?size={mag}&alloc=fresh"),
            Throughput::Bytes(size as u64),
            "fresh", &param, 20,
            |iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let mut v: Vec<u8> = Vec::with_capacity(size);
                    unsafe { v.set_len(size); }
                    v[0] = 0x42;
                    if size > 1 { v[size - 1] = 0x42; }
                    std::hint::black_box(&v);
                    drop(v);
                }
                start.elapsed()
            },
        );

        // reuse without zeroing
        {
            let mut v: Vec<u8> = vec![0u8; size];
            calibrated_bench(session, &mut group, "allocation_lifecycle",
                &format!("osc:lat/mem.alloc?size={mag}&alloc=reuse&zero=no"),
                Throughput::Bytes(size as u64),
                "reuse", &param, 20,
                |iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        v.clear();
                        unsafe { v.set_len(size); }
                        v[0] = 0x42;
                        if size > 1 { v[size - 1] = 0x42; }
                        std::hint::black_box(&v);
                    }
                    start.elapsed()
                },
            );
        }

        // reuse with zeroing (production ZeroizeOnDrop cost)
        {
            let mut v: Vec<u8> = vec![0u8; size];
            calibrated_bench(session, &mut group, "allocation_lifecycle",
                &format!("osc:lat/mem.alloc?size={mag}&alloc=reuse&zero=yes"),
                Throughput::Bytes(size as u64),
                "reuse_zeroed", &param, 20,
                |iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        v.fill(0);
                        v[0] = 0x42;
                        if size > 1 { v[size - 1] = 0x42; }
                        std::hint::black_box(&v);
                    }
                    start.elapsed()
                },
            );
        }
    }

    group.finish();
}

// ── Page fault floor ────────────────────────────────────────────────
// Line chart: function=cold_mmap/map_populate/prefaulted_reuse, parameter=size.
// Shows pool reuse advantage over fresh mmap.
// Throughput::Bytes(size) so as_number() returns distinct values per size
// (enabling the line chart) and CLI output shows "MiB/s" (bytes faulted/sec).
// SamplingMode::Flat because mmap/munmap is not linearly scalable.

fn page_fault(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("page_fault");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    group.sampling_mode(SamplingMode::Flat);
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let param = format!("{size}");
        let page_count = size / 4096;

        // mmap fresh — true page fault cost
        calibrated_bench(session, &mut group, "page_fault",
            &format!("osc:lat/mem.fault?size={mag}&state=cold"),
            Throughput::Bytes(size as u64),
            "cold_mmap", &param, 10,
            |iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let ptr = unsafe {
                        libc::mmap(std::ptr::null_mut(), size,
                            libc::PROT_READ | libc::PROT_WRITE,
                            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0)
                    };
                    assert_ne!(ptr, libc::MAP_FAILED);
                    let start = std::time::Instant::now();
                    let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, size) };
                    for i in 0..page_count { slice[i * 4096] = 0x42; }
                    std::hint::black_box(&slice[0]);
                    total += start.elapsed();
                    unsafe { libc::munmap(ptr, size); }
                }
                total
            },
        );

        // MAP_POPULATE — pre-faulted at mmap time
        calibrated_bench(session, &mut group, "page_fault",
            &format!("osc:lat/mem.fault?size={mag}&state=prefaulted"),
            Throughput::Bytes(size as u64),
            "map_populate", &param, 10,
            |iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = std::time::Instant::now();
                    let ptr = unsafe {
                        libc::mmap(std::ptr::null_mut(), size,
                            libc::PROT_READ | libc::PROT_WRITE,
                            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_POPULATE, -1, 0)
                    };
                    assert_ne!(ptr, libc::MAP_FAILED);
                    let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, size) };
                    for i in 0..page_count { slice[i * 4096] = 0x42; }
                    std::hint::black_box(&slice[0]);
                    total += start.elapsed();
                    unsafe { libc::munmap(ptr, size); }
                }
                total
            },
        );

        // Pre-faulted reuse — pool pattern
        {
            let mut v = vec![0u8; size];
            for i in 0..page_count { v[i * 4096] = 0x42; }
            calibrated_bench(session, &mut group, "page_fault",
                &format!("osc:lat/mem.fault?size={mag}&state=warm"),
                Throughput::Bytes(size as u64),
                "prefaulted_reuse", &param, 10,
                |iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        for i in 0..page_count { v[i * 4096] = 0x43; }
                        std::hint::black_box(&v);
                    }
                    start.elapsed()
                },
            );
        }
    }

    group.finish();
}

// ── blake3 hash scaling ─────────────────────────────────────────────
// Line chart: function=blake3, parameter=size.
// Shows where blake3 hits the DRAM cliff.

fn blake3(c: &mut Criterion, session: &mut CalibratedSession) {
    let mut group = c.benchmark_group("blake3_scaling");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536, 1024 * 1024, 16 * 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let osc = format!("osc:bw/crypto.hash?size={mag}&variant=blake3");
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();

        calibrated_bench(session, &mut group, "blake3_scaling", &osc,
            Throughput::Bytes(size as u64),
            "blake3", &format!("{size}"), 20,
            |iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    std::hint::black_box(blake3::hash(&data));
                }
                start.elapsed()
            },
        );
    }

    group.finish();
}
