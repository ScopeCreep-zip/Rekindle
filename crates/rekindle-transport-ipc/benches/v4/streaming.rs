//! v4 streaming benchmarks: SharedArena, slot lifecycle, tier selection,
//! codec roundtrips, concurrent access, and simulated production workloads.
//!
//! Every benchmark exercises real kernel primitives (memfd_create, mmap,
//! AtomicU64 CAS). No stubs, no mocks.
//!
//! # Benchmark Groups
//!
//! - `shared_mem_throughput/{frame_size}` — sustained write+read throughput
//! - `shared_mem_roundtrip/{frame_size}` — single frame acquire→write→publish→read→return
//! - `shared_mem_acquire_latency` — CAS-only slot acquire+release, no payload
//! - `shared_mem_concurrent/{num_streams}` — N threads writing 4K NV12 concurrently
//! - `shared_mem_drop_rate/{num_streams}` — frame drop measurement under contention
//! - `codec_roundtrip/{frame_type}` — encode+decode for all wire types
//! - `tier_selection_overhead` — select_tier() call overhead
//! - `integrity_overhead/{frame_size}` — BLAKE3 digest cost vs no-digest

use std::sync::Arc;
use std::time::Duration;
use criterion::{
    black_box, criterion_group, criterion_main,
    BenchmarkId, Criterion, Throughput,
};

#[cfg(target_os = "linux")]
use rekindle_transport_ipc::v4::streaming::shared_arena::{
    SharedArena, SharedMemRef, SlotRelease,
    NV12_4K_FRAME_SIZE, P010_4K_FRAME_SIZE, BGRA_4K_FRAME_SIZE,
};
#[cfg(target_os = "linux")]
use rekindle_transport_ipc::v4::streaming::tier;

use rekindle_transport_ipc::v4::streaming::dmabuf::{DmaBufRef, DmaBufPlane};
use rekindle_transport_ipc::v4::codec::streaming::{
    arena_write, slot_release, arena_setup, arena_ack, dmabuf_ref,
};

// ── Frame sizes for throughput benchmarks ───────────────────────

#[cfg(target_os = "linux")]
const FRAME_720P_NV12: usize = 1280 * 720 * 3 / 2;      // 1,382,400
#[cfg(target_os = "linux")]
const FRAME_1080P_NV12: usize = 1920 * 1080 * 3 / 2;    // 3,110,400
#[cfg(target_os = "linux")]
const FRAME_4K_NV12: usize = NV12_4K_FRAME_SIZE;         // 12,441,600
#[cfg(target_os = "linux")]
const FRAME_4K_BGRA: usize = BGRA_4K_FRAME_SIZE;         // 33,177,600

// ── Helpers ─────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn make_arena(slot_size: usize, slot_count: usize, integrity: bool) -> SharedArena {
    SharedArena::create(slot_size, slot_count, integrity)
        .expect("arena creation failed")
}

#[cfg(target_os = "linux")]
fn make_payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

// ── shared_mem_roundtrip ────────────────────────────────────────
//
// Full cycle: acquire → memcpy → publish → read_ref → return_slot.
// Measures the complete IPC transit time for a single frame.
// Pass threshold: < 1.5ms for 4K NV12 (spec §1.4).

#[cfg(target_os = "linux")]
fn shared_mem_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("shared_mem_roundtrip");

    for &(size, label) in &[
        (FRAME_720P_NV12, "720p_NV12"),
        (FRAME_1080P_NV12, "1080p_NV12"),
        (FRAME_4K_NV12, "4K_NV12"),
        (FRAME_4K_BGRA, "4K_BGRA"),
    ] {
        let arena = make_arena(size, 8, false);
        let payload = make_payload(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &size, |b, _| {
            b.iter(|| {
                // Writer: acquire → write → publish
                let mut guard = arena.try_acquire().expect("slot available");
                guard.write_from_slice(&payload);
                let shmref = guard.release(0, size);

                // Reader: validate → read → return
                let data = arena.read_ref(&shmref).expect("read_ref valid");
                black_box(data.len());

                // Writer: reclaim slot
                let release = SlotRelease {
                    arena_id: shmref.arena_id,
                    slot: shmref.slot,
                    generation: shmref.generation,
                };
                assert!(arena.return_slot(&release));
            });
        });
    }
    group.finish();
}

// ── shared_mem_throughput ───────────────────────────────────────
//
// Sustained: N iterations writing full frames as fast as possible.
// Reports GiB/s aggregate throughput.
// Pass threshold: > 8.5 GiB/s for 4K NV12 (spec §1.4, corrected).

#[cfg(target_os = "linux")]
fn shared_mem_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("shared_mem_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for &(size, label) in &[
        (FRAME_720P_NV12, "720p_NV12"),
        (FRAME_1080P_NV12, "1080p_NV12"),
        (FRAME_4K_NV12, "4K_NV12"),
        (FRAME_4K_BGRA, "4K_BGRA"),
    ] {
        let arena = make_arena(size, 8, false);
        let payload = make_payload(size);

        group.throughput(Throughput::Bytes(size as u64 * 120));
        group.bench_with_input(BenchmarkId::from_parameter(label), &size, |b, _| {
            b.iter(|| {
                for _ in 0..120 {
                    let mut guard = arena.try_acquire().expect("slot available");
                    guard.write_from_slice(&payload);
                    let shmref = guard.release(0, size);
                    let data = arena.read_ref(&shmref).expect("read_ref valid");
                    black_box(data.len());
                    let release = SlotRelease {
                        arena_id: 0,
                        slot: shmref.slot,
                        generation: shmref.generation,
                    };
                    arena.return_slot(&release);
                }
            });
        });
    }
    group.finish();
}

// ── shared_mem_acquire_latency ──────────────────────────────────
//
// CAS-only: try_acquire + immediate drop (rollback). No payload write.
// Measures raw slot acquisition overhead.
// Pass threshold: < 100ns median (spec §1.4).

#[cfg(target_os = "linux")]
fn shared_mem_acquire_latency(c: &mut Criterion) {
    let arena = make_arena(4096, 8, false);
    let mut group = c.benchmark_group("shared_mem_acquire_latency");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("10k_acquire_release", |b| {
        b.iter(|| {
            for _ in 0..10_000 {
                let guard = arena.try_acquire().expect("slot available");
                black_box(guard.slot_index());
                // Drop rolls back: ACQUIRED → FREE, no generation increment
            }
        });
    });
    group.finish();
}

// ── shared_mem_concurrent ───────────────────────────────────────
//
// N threads, each writing 4K NV12 as fast as possible.
// Measures aggregate throughput under contention.
// Pass threshold: > 15 GiB/s for 2 streams (spec §1.4, corrected).

#[cfg(target_os = "linux")]
fn shared_mem_concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("shared_mem_concurrent");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for &num_streams in &[1u8, 2, 4] {
        let slot_count = 8 * num_streams as usize;
        let arena = Arc::new(make_arena(FRAME_4K_NV12, slot_count, false));
        let payload = make_payload(FRAME_4K_NV12);

        group.throughput(Throughput::Bytes(FRAME_4K_NV12 as u64 * 60 * num_streams as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{num_streams}_streams")),
            &num_streams,
            |b, &n| {
                b.iter(|| {
                    let handles: Vec<_> = (0..n).map(|_| {
                        let arena = Arc::clone(&arena);
                        let payload = payload.clone();
                        std::thread::spawn(move || {
                            for _ in 0..60 {
                                if let Some(mut guard) = arena.try_acquire() {
                                    guard.write_from_slice(&payload);
                                    let shmref = guard.release(0, FRAME_4K_NV12);
                                    let data = arena.read_ref(&shmref).expect("read_ref");
                                    black_box(data.len());
                                    let release = SlotRelease {
                                        arena_id: 0,
                                        slot: shmref.slot,
                                        generation: shmref.generation,
                                    };
                                    arena.return_slot(&release);
                                }
                            }
                        })
                    }).collect();
                    for h in handles { h.join().unwrap(); }
                });
            },
        );
    }
    group.finish();
}

// ── shared_mem_drop_rate ────────────────────────────────────────
//
// Fixed arena (8 slots), variable stream count. Measures how many
// try_acquire() calls return None (frame drops) over 1000 attempts.

#[cfg(target_os = "linux")]
fn shared_mem_drop_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("shared_mem_drop_rate");
    group.sample_size(10);

    for &num_streams in &[1u8, 2, 4, 8] {
        let arena = Arc::new(make_arena(4096, 8, false));
        let payload = vec![0xAAu8; 4096];

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{num_streams}_streams_8_slots")),
            &num_streams,
            |b, &n| {
                b.iter(|| {
                    let attempts = Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let drops = Arc::new(std::sync::atomic::AtomicU64::new(0));

                    let handles: Vec<_> = (0..n).map(|_| {
                        let arena = Arc::clone(&arena);
                        let payload = payload.clone();
                        let attempts = Arc::clone(&attempts);
                        let drops = Arc::clone(&drops);
                        std::thread::spawn(move || {
                            for _ in 0..1000 {
                                attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                match arena.try_acquire() {
                                    Some(mut guard) => {
                                        guard.write_from_slice(&payload);
                                        let shmref = guard.release(0, 4096);
                                        if let Some(data) = arena.read_ref(&shmref) {
                                            black_box(data.len());
                                        }
                                        let release = SlotRelease {
                                            arena_id: 0,
                                            slot: shmref.slot,
                                            generation: shmref.generation,
                                        };
                                        arena.return_slot(&release);
                                    }
                                    None => {
                                        drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                            }
                        })
                    }).collect();
                    for h in handles { h.join().unwrap(); }

                    let total = attempts.load(std::sync::atomic::Ordering::Relaxed);
                    let dropped = drops.load(std::sync::atomic::Ordering::Relaxed);
                    let pct = if total > 0 { (dropped as f64 / total as f64) * 100.0 } else { 0.0 };
                    tracing::info!(
                        streams = n, total, dropped,
                        drop_pct = format_args!("{pct:.2}%"),
                        "drop rate measurement"
                    );
                    black_box(dropped);
                });
            },
        );
    }
    group.finish();
}

// ── integrity_overhead ──────────────────────────────────────────
//
// Compare roundtrip time with and without BLAKE3 integrity checking.
// Quantifies the cost of the optional digest computation.

#[cfg(target_os = "linux")]
fn integrity_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("integrity_overhead");

    for &(size, label) in &[
        (FRAME_1080P_NV12, "1080p_NV12"),
        (FRAME_4K_NV12, "4K_NV12"),
    ] {
        let payload = make_payload(size);

        for &integrity in &[false, true] {
            let arena = make_arena(size, 8, integrity);
            let suffix = if integrity { "with_blake3" } else { "no_digest" };

            group.throughput(Throughput::Bytes(size as u64));
            group.bench_with_input(
                BenchmarkId::new(label, suffix),
                &size,
                |b, _| {
                    b.iter(|| {
                        let mut guard = arena.try_acquire().expect("slot available");
                        guard.write_from_slice(&payload);
                        let shmref = guard.release(0, size);
                        let data = arena.read_ref(&shmref).expect("read_ref valid");
                        black_box(data.len());
                        let release = SlotRelease {
                            arena_id: 0,
                            slot: shmref.slot,
                            generation: shmref.generation,
                        };
                        arena.return_slot(&release);
                    });
                },
            );
        }
    }
    group.finish();
}

// ── codec_roundtrip ─────────────────────────────────────────────
//
// Encode + decode for every v4 streaming wire type.
// Measures serialization overhead per frame.

fn codec_roundtrip_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec_roundtrip");

    // SharedMemRef without digest (15 bytes)
    group.throughput(Throughput::Bytes(arena_write::WIRE_SIZE_NO_DIGEST as u64));
    group.bench_function("shared_mem_ref_no_digest", |b| {
        let shmref = SharedMemRef {
            arena_id: 0,
            slot: 7,
            generation: 42,
            offset: 0,
            length: 12_441_600,
            digest: None,
        };
        let mut buf = [0u8; 47];
        b.iter(|| {
            arena_write::encode(&shmref, &mut buf, false);
            let decoded = arena_write::decode(&buf[..15], false).unwrap();
            black_box(decoded);
        });
    });

    // SharedMemRef with digest (47 bytes)
    group.throughput(Throughput::Bytes(arena_write::WIRE_SIZE_WITH_DIGEST as u64));
    group.bench_function("shared_mem_ref_with_digest", |b| {
        let shmref = SharedMemRef {
            arena_id: 0,
            slot: 7,
            generation: 42,
            offset: 0,
            length: 12_441_600,
            digest: Some([0xAA; 32]),
        };
        let mut buf = [0u8; 47];
        b.iter(|| {
            arena_write::encode(&shmref, &mut buf, true);
            let decoded = arena_write::decode(&buf, true).unwrap();
            black_box(decoded);
        });
    });

    // SlotRelease (7 bytes)
    group.throughput(Throughput::Bytes(slot_release::WIRE_SIZE as u64));
    group.bench_function("slot_release", |b| {
        let release = SlotRelease { arena_id: 0, slot: 7, generation: 42 };
        let mut buf = [0u8; 7];
        b.iter(|| {
            slot_release::encode(&release, &mut buf);
            let decoded = slot_release::decode(&buf).unwrap();
            black_box(decoded);
        });
    });

    // ArenaSetup (10 bytes)
    group.throughput(Throughput::Bytes(arena_setup::WIRE_SIZE as u64));
    group.bench_function("arena_setup", |b| {
        let setup = arena_setup::ArenaSetupPayload {
            slot_size: 16 * 1024 * 1024,
            slot_count: 8,
            integrity: 1,
        };
        b.iter(|| {
            let buf = arena_setup::encode(&setup);
            let decoded = arena_setup::decode(&buf).unwrap();
            black_box(decoded);
        });
    });

    // ArenaAck (1 byte)
    group.throughput(Throughput::Bytes(arena_ack::WIRE_SIZE as u64));
    group.bench_function("arena_ack", |b| {
        b.iter(|| {
            let buf = arena_ack::encode(arena_ack::STATUS_OK);
            let decoded = arena_ack::decode(&buf).unwrap();
            black_box(decoded);
        });
    });

    // DmaBufRef (65 bytes)
    group.throughput(Throughput::Bytes(dmabuf_ref::WIRE_SIZE as u64));
    group.bench_function("dmabuf_ref", |b| {
        let dbr = DmaBufRef {
            pts: 1_000_000_000,
            width: 3840,
            height: 2160,
            fourcc: 0x3231564E,
            modifier: 0,
            num_planes: 2,
            _pad: [0; 3],
            planes: [
                DmaBufPlane { stride: 3840, offset: 0 },
                DmaBufPlane { stride: 3840, offset: 3840 * 2160 },
                DmaBufPlane::default(),
                DmaBufPlane::default(),
            ],
            payload_id_hint: 1,
        };
        let mut buf = [0u8; 65];
        b.iter(|| {
            dmabuf_ref::encode(&dbr, &mut buf);
            let decoded = dmabuf_ref::decode(&buf).unwrap();
            black_box(decoded);
        });
    });

    group.finish();
}

// ── tier_selection_overhead ─────────────────────────────────────
//
// select_tier() call overhead for various payload sizes.
// Must be negligible — this runs on every send.

#[cfg(target_os = "linux")]
fn tier_selection_overhead(c: &mut Criterion) {
    let arena = make_arena(16 * 1024 * 1024, 8, false);
    let mut group = c.benchmark_group("tier_selection_overhead");
    group.throughput(Throughput::Elements(100_000));

    for &(size, label) in &[
        (256, "256B_inline"),
        (65536, "64KiB_inline_boundary"),
        (65537, "64KiB+1_shared_mem"),
        (FRAME_4K_NV12, "4K_NV12_shared_mem"),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(label), &size, |b, &sz| {
            b.iter(|| {
                for _ in 0..100_000 {
                    let t = tier::select_tier(sz, Some(&arena), false);
                    black_box(t);
                }
            });
        });
    }

    // DmaBuf path
    group.bench_function("dmabuf_fd_present", |b| {
        b.iter(|| {
            for _ in 0..100_000 {
                let t = tier::select_tier(FRAME_4K_NV12, Some(&arena), true);
                black_box(t);
            }
        });
    });

    group.finish();
}

// ── reclaim_all_inflight ────────────────────────────────────────
//
// Measures crash recovery path: all slots IN_FLIGHT, reclaim_all_inflight.

#[cfg(target_os = "linux")]
fn reclaim_all_inflight_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("reclaim_all_inflight");

    for &slot_count in &[4usize, 8, 16, 32] {
        let arena = make_arena(4096, slot_count, false);
        let payload = vec![0u8; 4096];

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{slot_count}_slots")),
            &slot_count,
            |b, _| {
                b.iter(|| {
                    // Put all slots into IN_FLIGHT
                    let mut refs = Vec::with_capacity(slot_count);
                    for _ in 0..slot_count {
                        let mut guard = arena.try_acquire().expect("slot available");
                        guard.write_from_slice(&payload);
                        refs.push(guard.release(0, 4096));
                    }
                    assert!(arena.try_acquire().is_none());

                    // Simulate crash: reclaim all
                    arena.reclaim_all_inflight();

                    // Verify all slots free
                    for _ in 0..slot_count {
                        let guard = arena.try_acquire().expect("slot should be free after reclaim");
                        drop(guard);
                    }
                });
            },
        );
    }
    group.finish();
}

// ── generation_churn ────────────────────────────────────────────
//
// Rapid acquire→publish→return on a single slot. Measures generation
// counter advancement and CAS overhead under sequential hot-path use.

#[cfg(target_os = "linux")]
fn generation_churn(c: &mut Criterion) {
    let arena = make_arena(4096, 1, false);
    let payload = vec![0xBBu8; 4096];
    let mut group = c.benchmark_group("generation_churn");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("10k_cycles_single_slot", |b| {
        b.iter(|| {
            for _ in 0..10_000 {
                let mut guard = arena.try_acquire().expect("slot available");
                guard.write_from_slice(&payload);
                let shmref = guard.release(0, 4096);
                let data = arena.read_ref(&shmref).expect("read_ref");
                black_box(data[0]);
                let release = SlotRelease {
                    arena_id: 0,
                    slot: shmref.slot,
                    generation: shmref.generation,
                };
                arena.return_slot(&release);
            }
        });
    });
    group.finish();
}

// ── Criterion groups ────────────────────────────────────────────

#[cfg(target_os = "linux")]
criterion_group!(
    arena_benches,
    shared_mem_roundtrip,
    shared_mem_throughput,
    shared_mem_acquire_latency,
    shared_mem_concurrent,
    shared_mem_drop_rate,
    integrity_overhead,
    reclaim_all_inflight_bench,
    generation_churn,
    tier_selection_overhead,
);

criterion_group!(
    codec_benches,
    codec_roundtrip_bench,
);

#[cfg(target_os = "linux")]
criterion_main!(arena_benches, codec_benches);

#[cfg(not(target_os = "linux"))]
criterion_main!(codec_benches);
