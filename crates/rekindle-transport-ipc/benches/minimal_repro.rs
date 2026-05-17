//! Minimal reproduction: does blocking_recv work in a criterion bench binary?
//!
//! If this hangs, blocking_recv is fundamentally broken in criterion bench binaries.
//! If this passes, the issue is in send_payload / rayon / BufferPool interaction.

use criterion::{criterion_group, criterion_main, Criterion};
use tokio::sync::mpsc;

fn repro_blocking_recv(c: &mut Criterion) {
    c.bench_function("blocking_recv_minimal", |b| {
        b.iter(|| {
            let (tx, mut rx) = mpsc::channel::<u8>(16);
            std::thread::spawn(move || {
                tx.blocking_send(42).unwrap();
                // tx dropped here — channel closes
            });
            let val = rx.blocking_recv();
            assert_eq!(val, Some(42));
            let none = rx.blocking_recv();
            assert_eq!(none, None);
        });
    });
}

fn repro_rayon_blocking_send(c: &mut Criterion) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();

    c.bench_function("rayon_blocking_send_minimal", |b| {
        b.iter(|| {
            let (tx, mut rx) = mpsc::channel::<u8>(16);
            pool.spawn(move || {
                tx.blocking_send(99).unwrap();
                // tx dropped here — channel closes
            });
            let val = rx.blocking_recv();
            assert_eq!(val, Some(99));
            let none = rx.blocking_recv();
            assert_eq!(none, None);
        });
    });
}

fn repro_rayon_many_tasks(c: &mut Criterion) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();

    c.bench_function("rayon_257_tasks", |b| {
        b.iter(|| {
            let (tx, mut rx) = mpsc::channel::<u8>(64);
            for i in 0..257u16 {
                let tx_clone = tx.clone();
                pool.spawn(move || {
                    tx_clone.blocking_send(i as u8).unwrap();
                });
            }
            drop(tx); // drop original — only task clones remain

            // Drain on a separate thread (consumer before producer completes)
            let consumer = std::thread::spawn(move || {
                let mut count = 0u32;
                while let Some(_) = rx.blocking_recv() {
                    count += 1;
                }
                count
            });
            let count = consumer.join().unwrap();
            assert_eq!(count, 257);
        });
    });
}

/// Level 4: rayon tasks + BufferPool (no send_payload, manual encrypt simulation)
fn repro_rayon_with_pool(c: &mut Criterion) {
    use rekindle_transport_ipc::bulk::pool::BufferPool;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let buffer_pool = BufferPool::new(512);

    c.bench_function("rayon_pool_257_tasks", |b| {
        b.iter(|| {
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
            let bp = buffer_pool.clone();
            for _ in 0..257u16 {
                let tx_clone = tx.clone();
                let bp_clone = bp.clone();
                pool.spawn(move || {
                    let mut slab = bp_clone.acquire();
                    slab.extend_from_slice(&[0xAB; 1024]);
                    tx_clone.blocking_send(slab).unwrap();
                });
            }
            drop(tx);

            let consumer = std::thread::spawn(move || {
                let mut count = 0u32;
                while let Some(frame) = rx.blocking_recv() {
                    // Replenish slab back to pool — simulates write task behavior
                    bp.replenish(frame);
                    count += 1;
                }
                count
            });
            let count = consumer.join().unwrap();
            assert_eq!(count, 257);
        });
    });
}

/// Level 5: rayon tasks + BufferPool WITH replenish in consumer
/// (Previously "no_replenish" which hangs after ~15 iterations — pool exhaustion confirmed)
fn repro_rayon_pool_no_replenish(c: &mut Criterion) {
    use rekindle_transport_ipc::bulk::pool::BufferPool;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let buffer_pool = BufferPool::new(512);

    c.bench_function("rayon_pool_replenish_257", |b| {
        b.iter(|| {
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
            let bp = buffer_pool.clone();
            let bp_consumer = buffer_pool.clone();
            for _ in 0..257u16 {
                let tx_clone = tx.clone();
                let bp_clone = bp.clone();
                pool.spawn(move || {
                    let mut slab = bp_clone.acquire();
                    slab.extend_from_slice(&[0xAB; 1024]);
                    tx_clone.blocking_send(slab).unwrap();
                });
            }
            drop(tx);

            let consumer = std::thread::spawn(move || {
                let mut count = 0u32;
                while let Some(slab) = rx.blocking_recv() {
                    bp_consumer.replenish(slab);
                    count += 1;
                }
                count
            });
            let count = consumer.join().unwrap();
            assert_eq!(count, 257);
        });
    });
}

/// Level 6: actual send_payload (the real function) — WITH replenish
fn repro_send_payload(c: &mut Criterion) {
    use std::sync::Arc;
    use rekindle_transport_ipc::bulk::{
        cipher::BulkCipher,
        encrypt::build_encrypt_pool,
        nonce::NonceCounter,
        pool::BufferPool,
        transfer::send_payload,
        verify::DigestAlgorithm,
    };

    let encrypt_pool = build_encrypt_pool(0);
    let buffer_pool = BufferPool::new(512);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let payload = vec![0xABu8; 1024]; // tiny: 1 data chunk + 1 fin = 2 tasks

    c.bench_function("send_payload_1kb", |b| {
        b.iter(|| {
            let nonce_ctr = Arc::new(NonceCounter::new());
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
            let bp = Arc::clone(&buffer_pool);
            let consumer = std::thread::spawn(move || {
                let mut frames = Vec::new();
                while let Some(f) = rx.blocking_recv() { frames.push(f); }
                // Replenish slabs — without this, pool exhausts after N iterations
                for slab in frames.drain(..) { bp.replenish(slab); }
                frames
            });
            send_payload(&encrypt_pool, &cipher, &nonce_ctr, &buffer_pool, tx, 0, &payload, DigestAlgorithm::Blake3);
            let frames = consumer.join().unwrap();
            assert_eq!(frames.len(), 0); // drained by replenish
        });
    });
}

criterion_group!(
    benches,
    repro_blocking_recv,
    repro_rayon_blocking_send,
    repro_rayon_many_tasks,
    repro_rayon_with_pool,
    repro_rayon_pool_no_replenish,
    repro_send_payload,
);
criterion_main!(benches);
