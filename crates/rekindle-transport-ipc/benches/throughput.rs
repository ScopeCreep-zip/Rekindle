//! Comprehensive benchmarks: throughput, latency, scaling, components.
//!
//! Every benchmark uses Throughput::Bytes for direct GiB/s reporting
//! or measures latency in ns per operation.

use std::sync::Arc;
use std::time::Duration;
use criterion::{criterion_group, criterion_main, Criterion, Throughput, BenchmarkId};
use tokio::sync::mpsc;

use rekindle_transport_ipc::backpressure::GlobalMemoryGuard;
use rekindle_transport_ipc::bulk::{
    BulkCounters,
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk, DEFAULT_REASSEMBLY_CAPACITY},
    encrypt::build_encrypt_pool,
    frame::{MAX_CHUNK_PLAIN, HEADER_LEN, TAG_LEN},
    nonce::NonceCounter,
    pool::BufferPool,
    reassembly::Reassembler,
    replay::ReplayFilter,
    transfer::{send_payload, BulkTransferAccumulator},
    verify::DigestAlgorithm,
};

// ---- 11.1 Bulk E2E throughput (GiB/s) ----

fn bench_bulk_e2e(c: &mut Criterion) {
    let encrypt_pool = build_encrypt_pool(0);
    let decrypt_pool = build_encrypt_pool(0);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));

    let total_bytes = 256 * MAX_CHUNK_PLAIN;
    let payload: Vec<u8> = (0..total_bytes).map(|i| (i % 251) as u8).collect();

    let mut group = c.benchmark_group("bulk_e2e");
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.measurement_time(Duration::from_secs(20));

    // Pool allocated ONCE outside b.iter() — matches production lifecycle.
    // Consumer replenishes slabs inside b.iter() — measures only crypto, not allocation.
    let buffer_pool = BufferPool::new(512);

    group.bench_function("blake3_16MiB", |b| {
        b.iter(|| {
            // Phase 1: Encrypt.
            let nonce_ctr = Arc::new(NonceCounter::new());
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
            let bp = Arc::clone(&buffer_pool);
            let consumer = std::thread::spawn(move || {
                let mut frames = Vec::new();
                while let Some(f) = rx.blocking_recv() {
                    frames.push(f);
                }
                // Replenish ALL slabs back to pool — matches production write task.
                // Without this, pool exhausts after ~2 iterations and acquire() spins forever.
                for slab in frames.drain(..) {
                    bp.replenish(slab);
                }
                frames
            });
            send_payload(&encrypt_pool, &cipher, &nonce_ctr, &buffer_pool, tx, 0, &payload, DigestAlgorithm::Blake3);
            let frames = consumer.join().unwrap();

            // Phase 2: Decrypt + reassemble. Same pattern: consumer first.
            let (dtx, mut drx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
            let decrypt_consumer = std::thread::spawn(move || {
                let mut reassembler = Reassembler::new(1024);
                let mut acc = BulkTransferAccumulator::new(0);
                while let Some(chunk) = drx.blocking_recv() {
                    for r in reassembler.process(chunk).unwrap() { acc.push(&r); }
                    if acc.is_complete() { break; }
                }
            });
            let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), Arc::clone(&decrypt_pool), dtx, DigestAlgorithm::Blake3, BulkCounters::new());
            for f in frames { dispatcher.dispatch(f).unwrap(); }
            drop(dispatcher);
            decrypt_consumer.join().unwrap();
        });
    });
    group.finish();
}

// ---- 11.2 Noise roundtrip ----

fn bench_noise_roundtrip(c: &mut Criterion) {
    use rekindle_transport_ipc::noise::keys::{generate_keypair, NOISE_PARAMS};
    use rekindle_transport_ipc::noise::resolver::noise_builder;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut init = noise_builder(NOISE_PARAMS).local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap().prologue(b"BENCH").unwrap().build_initiator().unwrap();
    let mut resp = noise_builder(NOISE_PARAMS).local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"BENCH").unwrap().build_responder().unwrap();
    let mut buf = [0u8; 256]; let mut pay = [0u8; 256];
    let len = init.write_message(&[], &mut buf).unwrap();
    resp.read_message(&buf[..len], &mut pay).unwrap();
    let len = resp.write_message(&[], &mut buf).unwrap();
    init.read_message(&buf[..len], &mut pay).unwrap();
    let h_i = init.into_stateless_transport_mode().unwrap();
    let h_r = resp.into_stateless_transport_mode().unwrap();

    let msg = b"hello noise benchmark payload for realistic control frame";
    let msg_size = msg.len();
    let mut enc = vec![0u8; msg_size + 16];
    let mut dec = vec![0u8; msg_size];
    let mut nonce = 0u64;

    let mut group = c.benchmark_group("noise_crypto");
    group.throughput(Throughput::Bytes((msg_size * 2) as u64));
    group.measurement_time(Duration::from_secs(15));
    group.bench_function(format!("roundtrip_{msg_size}B"), |b| {
        b.iter(|| {
            nonce += 1;
            let len = h_i.write_message(nonce, msg, &mut enc).unwrap();
            h_r.read_message(nonce, &enc[..len], &mut dec).unwrap();
        });
    });
    group.finish();
}

// ---- 11.3 AES-GCM seal at multiple chunk sizes ----

fn bench_aes_seal(c: &mut Criterion) {
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("aes_gcm_seal");
    group.measurement_time(Duration::from_secs(15));
    for &size in &[1024usize, 4096, 16_384, 65_519, 262_144, 1_048_576] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let plain = vec![0xABu8; n];
            let mut buf = vec![0u8; n];
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                buf[..n].copy_from_slice(&plain);
                cipher.seal_in_place(nonce, b"aad", &mut buf).unwrap();
            });
        });
    }
    group.finish();
}

// ---- 11.6 Small message latency ----

fn bench_small_message(c: &mut Criterion) {
    use rekindle_transport_ipc::noise::keys::{generate_keypair, NOISE_PARAMS};
    use rekindle_transport_ipc::noise::resolver::noise_builder;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let mut init = noise_builder(NOISE_PARAMS).local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap().prologue(b"SMALL").unwrap().build_initiator().unwrap();
    let mut resp = noise_builder(NOISE_PARAMS).local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"SMALL").unwrap().build_responder().unwrap();
    let mut buf = [0u8; 256]; let mut pay = [0u8; 256];
    let len = init.write_message(&[], &mut buf).unwrap();
    resp.read_message(&buf[..len], &mut pay).unwrap();
    let len = resp.write_message(&[], &mut buf).unwrap();
    init.read_message(&buf[..len], &mut pay).unwrap();
    let h_i = init.into_stateless_transport_mode().unwrap();
    let h_r = resp.into_stateless_transport_mode().unwrap();

    let mut group = c.benchmark_group("small_message");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[16usize, 64, 128, 256, 512, 1024] {
        let msg = vec![0xAAu8; size];
        let mut enc = vec![0u8; size + 16];
        let mut dec = vec![0u8; size];
        let mut nonce = 0u64;

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("encrypt_decrypt", size), &size, |b, _| {
            b.iter(|| {
                nonce += 1;
                let len = h_i.write_message(nonce, &msg, &mut enc).unwrap();
                h_r.read_message(nonce, &enc[..len], &mut dec).unwrap();
            });
        });
    }
    group.finish();
}

// ---- 11.7 Bulk throughput at multiple payload sizes ----

fn bench_bulk_sizes(c: &mut Criterion) {
    let encrypt_pool = build_encrypt_pool(0);
    let decrypt_pool = build_encrypt_pool(0);
    let buffer_pool = BufferPool::new(512);
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));

    let mut group = c.benchmark_group("bulk_sizes");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[1024usize, 65_519, 1_000_000, 16_000_000] {
        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                let nonce = Arc::new(NonceCounter::new());
                let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
                let bp = Arc::clone(&buffer_pool);
                let consumer = std::thread::spawn(move || {
                    let mut f = Vec::new();
                    while let Some(frame) = rx.blocking_recv() { f.push(frame); }
                    for slab in f.drain(..) { bp.replenish(slab); }
                    f
                });
                send_payload(&encrypt_pool, &cipher, &nonce, &buffer_pool, tx, 0, &payload, DigestAlgorithm::Blake3);
                let frames = consumer.join().unwrap();

                let (dtx, mut drx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
                let decrypt_consumer = std::thread::spawn(move || {
                    let mut r = Reassembler::new(1024);
                    let mut a = BulkTransferAccumulator::new(0);
                    while let Some(chunk) = drx.blocking_recv() {
                        for rc in r.process(chunk).unwrap() { a.push(&rc); }
                        if a.is_complete() { break; }
                    }
                });
                let mut d = BulkDispatcher::new(Arc::clone(&cipher), Arc::clone(&decrypt_pool), dtx, DigestAlgorithm::Blake3, BulkCounters::new());
                for f in frames { d.dispatch(f).unwrap(); }
                drop(d);
                decrypt_consumer.join().unwrap();
            });
        });
    }
    group.finish();
}

// ---- 11.10 Handshake rate (handshakes/sec) ----

fn bench_handshake_rate(c: &mut Criterion) {
    use rekindle_transport_ipc::noise::keys::{generate_keypair, NOISE_PARAMS};
    use rekindle_transport_ipc::noise::resolver::noise_builder;

    let mut group = c.benchmark_group("handshake");
    group.measurement_time(Duration::from_secs(15));
    group.bench_function("full_ik_handshake", |b| {
        b.iter(|| {
            let skp = generate_keypair().unwrap();
            let ckp = generate_keypair().unwrap();
            let spub: [u8; 32] = skp.public().try_into().unwrap();

            let mut init = noise_builder(NOISE_PARAMS).local_private_key(&ckp.as_inner().private).unwrap()
                .remote_public_key(&spub).unwrap().prologue(b"HS").unwrap().build_initiator().unwrap();
            let mut resp = noise_builder(NOISE_PARAMS).local_private_key(&skp.as_inner().private).unwrap()
                .prologue(b"HS").unwrap().build_responder().unwrap();
            let mut buf = [0u8; 256]; let mut pay = [0u8; 256];
            let len = init.write_message(&[], &mut buf).unwrap();
            resp.read_message(&buf[..len], &mut pay).unwrap();
            let len = resp.write_message(&[], &mut buf).unwrap();
            init.read_message(&buf[..len], &mut pay).unwrap();
            let _ = init.into_stateless_transport_mode().unwrap();
            let _ = resp.into_stateless_transport_mode().unwrap();
        });
    });
    group.finish();
}

// ---- 11.12 Buffer pool cycle rate ----

fn bench_pool_cycle(c: &mut Criterion) {
    let pool = BufferPool::new(256);
    let mut group = c.benchmark_group("buffer_pool");
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("acquire_replenish", |b| {
        b.iter(|| {
            let slab = pool.acquire();
            pool.replenish(slab);
        });
    });
    group.finish();
}

// ---- 11.13 AES-GCM decrypt throughput ----

fn bench_aes_decrypt(c: &mut Criterion) {
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut group = c.benchmark_group("aes_gcm_open");
    group.measurement_time(Duration::from_secs(15));

    for &size in &[1024usize, 4096, 16_384, 65_519] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            // Pre-allocate a reusable buffer — same as production (PooledBuf).
            // seal_in_place_append_tag + open_in_place on the same buffer:
            // zero intermediate allocation, symmetric with the seal bench.
            let plain = vec![0xABu8; n];
            let mut buf = Vec::with_capacity(n + 16);
            let mut nonce = 0u64;
            b.iter(|| {
                nonce = nonce.wrapping_add(1);
                // Reset buffer to plaintext (simulates receiving a fresh chunk)
                buf.clear();
                buf.extend_from_slice(&plain);
                // Seal + append tag in one call — no separate tag, no copy
                cipher.seal_in_place_append_tag(nonce, b"aad", &mut buf).unwrap();
                // Open in-place — same buffer, no allocation
                cipher.open_in_place(nonce, b"aad", &mut buf).unwrap();
            });
        });
    }
    group.finish();
}

// ---- 11.14 Reassembly throughput (isolated) ----
//
// Measures reassembly + Merkle verification without decrypt overhead.
// Protocol-correct: 256 BulkData chunks followed by 1 BulkFin chunk.
// BulkFin has empty plaintext and carries the merkle root as fin_digest.

fn bench_reassembly(c: &mut Criterion) {
    use rekindle_transport_ipc::bulk::pool::ZeroizingBuf;
    use rekindle_transport_ipc::bulk::verify::digest_oneshot;

    let algo = DigestAlgorithm::Blake3;
    let chunk_size = MAX_CHUNK_PLAIN;
    let num_data_chunks = 256usize;

    let mut group = c.benchmark_group("reassembly");
    group.throughput(Throughput::Bytes((num_data_chunks * chunk_size) as u64));
    group.measurement_time(Duration::from_secs(15));

    // Pre-compute per-chunk digest and merkle root.
    let data = vec![0xABu8; chunk_size];
    let chunk_digest = digest_oneshot(algo, &data);
    let merkle_input: Vec<u8> = (0..num_data_chunks)
        .flat_map(|_| chunk_digest.iter().copied())
        .collect();
    let merkle_root = digest_oneshot(algo, &merkle_input);

    group.bench_function("256_chunks_blake3", |b| {
        b.iter(|| {
            let mut reassembler = Reassembler::with_algorithm(2048, algo);
            let mut acc = BulkTransferAccumulator::new(0);

            // 256 BulkData chunks (chunk_seq 0-255).
            for i in 0..num_data_chunks {
                let chunk = DecryptedChunk {
                    stream_id: 0,
                    chunk_seq: i as u32,
                    plaintext: ZeroizingBuf::new(data.clone()),
                    is_last: false,
                    fin_digest: None,
                    chunk_digest,
                    decrypt_failed: false,
                    reservation: None,
                    per_conn_reservation: None,
                };
                let delivered = reassembler.process(chunk).unwrap();
                for r in &delivered { acc.push(r); }
            }

            // 1 BulkFin chunk (chunk_seq 256): empty plaintext, carries merkle root.
            let fin_chunk = DecryptedChunk {
                stream_id: 0,
                chunk_seq: num_data_chunks as u32,
                plaintext: ZeroizingBuf::new(Vec::new()),
                is_last: true,
                fin_digest: Some(merkle_root),
                chunk_digest: [0u8; 32],
                decrypt_failed: false,
                reservation: None,
                per_conn_reservation: None,
            };
            let delivered = reassembler.process(fin_chunk).unwrap();
            for r in &delivered { acc.push(r); }

            assert!(acc.is_complete());
        });
    });
    group.finish();
}

// ---- 11.15 Dispatcher throughput (isolated) ----
//
// Measures frame parse + replay filter + rayon submit overhead without
// socket I/O. Constructs real encrypted wire frames [HEADER_LEN + ct + TAG_LEN]
// and dispatches them through BulkDispatcher.

fn bench_dispatcher(c: &mut Criterion) {
    use rekindle_transport_ipc::bulk::frame::{BulkFrameHeader, FrameKind};

    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let decrypt_pool = build_encrypt_pool(0);
    let num_frames = 256usize;
    let chunk_size = MAX_CHUNK_PLAIN;

    // Pre-build encrypted frames.
    let mut frames = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let header = BulkFrameHeader::new(0, FrameKind::BulkData, i as u64, i as u32);
        let hdr = header.encode_array();
        let plaintext = vec![0xABu8; chunk_size];
        let mut frame = Vec::with_capacity(HEADER_LEN + chunk_size + TAG_LEN);
        frame.extend_from_slice(&hdr);
        frame.extend_from_slice(&plaintext);
        let ct_start = HEADER_LEN;
        let tag = cipher
            .seal_in_place(i as u64, &hdr, &mut frame[ct_start..ct_start + chunk_size])
            .unwrap();
        frame.extend_from_slice(&tag);
        frames.push(frame);
    }

    let mut group = c.benchmark_group("dispatcher");
    group.throughput(Throughput::Bytes((num_frames * chunk_size) as u64));
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("256_frames_dispatch_decrypt", |b| {
        b.iter(|| {
            let (tx, mut drx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);
            let mut dispatcher = BulkDispatcher::new(
                Arc::clone(&cipher), Arc::clone(&decrypt_pool), tx,
                DigestAlgorithm::Blake3, BulkCounters::new(),
            );

            for frame in &frames {
                dispatcher.dispatch(frame.clone()).unwrap();
            }
            drop(dispatcher);

            let decrypt_consumer = std::thread::spawn(move || {
                let mut count = 0u64;
                while let Some(_chunk) = drx.blocking_recv() {
                    count += 1;
                }
                count
            });
            let count = decrypt_consumer.join().unwrap();
            assert_eq!(count, num_frames as u64);
        });
    });
    group.finish();
}

// ---- 11.10b Replay filter throughput ----

fn bench_replay_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("replay_filter");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("sequential", |b| {
        let mut filter = ReplayFilter::new();
        let mut nonce = 0u64;
        b.iter(|| {
            nonce += 1;
            filter.check_and_accept(nonce).unwrap();
        });
    });

    group.bench_function("random_within_window", |b| {
        let mut filter = ReplayFilter::new();
        // Advance to a high base.
        filter.check_and_accept(100_000).unwrap();
        let mut i = 99_001u64;
        b.iter(|| {
            i += 1;
            if i >= 100_000 { i = 99_001; filter = ReplayFilter::new(); filter.check_and_accept(100_000).unwrap(); }
            let _ = filter.check_and_accept(i);
        });
    });
    group.finish();
}

// ---- 11.12b Memory guard CAS throughput ----

fn bench_memory_guard(c: &mut Criterion) {
    let guard = Arc::new(GlobalMemoryGuard::new(u64::MAX)); // no limit pressure
    let mut group = c.benchmark_group("memory_guard");
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("reserve_release", |b| {
        b.iter(|| {
            let r = guard.try_reserve(1).unwrap();
            drop(r);
        });
    });
    group.finish();
}

// ---- Socket-level control frame roundtrip latency ----
//
// Upstream reference: snowstorm/benches/stress.rs:38-66
// Real server + real client + real Unix socket + real Noise handshake.
// Measures steady-state send_frame latency including kernel I/O,
// Noise encrypt/decrypt, codec framing, mpsc channels, and tokio scheduling.

fn bench_socket_roundtrip(c: &mut Criterion) {
    use rekindle_transport_ipc::client::IpcClient;
    use rekindle_transport_ipc::config::IpcConfig;
    use rekindle_transport_ipc::noise::keys::generate_keypair;
    use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
    use rekindle_transport_ipc::server::state::ServerState;
    use rekindle_transport_ipc::transport_frame::ConnectionPhase;

    struct BenchRouter;
    impl FrameRouter for BenchRouter {
        fn route_frame(&self, _: &ServerState, _: u64, _: bytes::Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let path = std::env::temp_dir().join(format!(
        "rekindle-bench-socket-{}.sock", std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let client = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), BenchRouter, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let ckp = generate_keypair().unwrap();
        let c = IpcClient::connect(
            uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
        ).await.unwrap();

        // Warmup: 100 frames to fill caches.
        for _ in 0..100 {
            c.send_frame(b"warmup", std::time::Duration::from_secs(5)).await;
        }
        c
    });

    let mut group = c.benchmark_group("socket_roundtrip");
    group.measurement_time(std::time::Duration::from_secs(15));

    // rt.block_on inside b.iter adds ~100ns runtime entry overhead.
    // For socket roundtrips at 10-100µs, this is 0.1-1% noise. Acceptable.
    for &size in &[16usize, 64, 256, 1024, 4096] {
        let payload = vec![0xAA; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("send_frame", size),
            &payload,
            |b, p| {
                b.iter(|| {
                    rt.block_on(async {
                        let outcome = client.send_frame(p, std::time::Duration::from_secs(5)).await;
                        assert!(outcome.is_delivered());
                    });
                });
            },
        );
    }
    group.finish();

    rt.block_on(async { client.shutdown().await });
    let _ = std::fs::remove_file(&path);
}

// ---- Concurrent control-plane throughput scaling ----

fn bench_concurrent_control(c: &mut Criterion) {
    use rekindle_transport_ipc::client::IpcClient;
    use rekindle_transport_ipc::config::IpcConfig;
    use rekindle_transport_ipc::noise::keys::generate_keypair;
    use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
    use rekindle_transport_ipc::server::state::ServerState;
    use rekindle_transport_ipc::transport_frame::ConnectionPhase;

    struct NR;
    impl FrameRouter for NR {
        fn route_frame(&self, _: &ServerState, _: u64, _: bytes::Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().build().unwrap();

    let path = std::env::temp_dir().join(format!(
        "rekindle-bench-cc-{}.sock", std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    // Setup server + 8 clients (max we'll use). Wrap in Arc for spawn.
    let clients: Vec<std::sync::Arc<IpcClient>> = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pub_key: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), NR, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;

        let mut cs = Vec::new();
        for _ in 0..8 {
            let ckp = generate_keypair().unwrap();
            let c = IpcClient::connect(
                uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
            ).await.unwrap();
            for _ in 0..10 { c.send_frame(b"w", std::time::Duration::from_secs(5)).await; }
            cs.push(std::sync::Arc::new(c));
        }
        cs
    });

    let payload = vec![0xBB; 256];
    let mut group = c.benchmark_group("concurrent_control");
    group.measurement_time(std::time::Duration::from_secs(15));

    for &n in &[1usize, 2, 4, 8] {
        group.throughput(Throughput::Elements(n as u64 * 10));
        group.bench_with_input(
            BenchmarkId::new("clients", n),
            &n,
            |b, &num| {
                b.iter(|| {
                    rt.block_on(async {
                        let mut tasks = Vec::new();
                        for client in clients.iter().take(num) {
                            let c = std::sync::Arc::clone(client);
                            let p = payload.clone();
                            tasks.push(tokio::spawn(async move {
                                for _ in 0..10 {
                                    c.send_frame(&p, std::time::Duration::from_secs(5)).await;
                                }
                            }));
                        }
                        for t in tasks { let _ = t.await; }
                    });
                });
            },
        );
    }
    group.finish();

    // Shutdown requires ownership — drop Arcs.
    // Can't call shutdown through Arc. IO loop exits when all senders drop.
    drop(clients);
    let _ = std::fs::remove_file(&path);
}

// ---- Handshake latency: full connect → send → shutdown per iteration ----

fn bench_handshake_latency(c: &mut Criterion) {
    use rekindle_transport_ipc::client::IpcClient;
    use rekindle_transport_ipc::config::IpcConfig;
    use rekindle_transport_ipc::noise::keys::generate_keypair;
    use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
    use rekindle_transport_ipc::server::state::ServerState;
    use rekindle_transport_ipc::transport_frame::ConnectionPhase;

    struct NR;
    impl FrameRouter for NR {
        fn route_frame(&self, _: &ServerState, _: u64, _: bytes::Bytes) {}
        fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
        fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
        fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().build().unwrap();

    let path = std::env::temp_dir().join(format!(
        "rekindle-bench-hs-{}.sock", std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let pub_key = rt.block_on(async {
        let kp = generate_keypair().unwrap();
        let pk: [u8; 32] = kp.public().try_into().unwrap();
        let server = IpcServer::bind(&path, kp.into_inner(), NR, IpcConfig::default()).unwrap();
        tokio::spawn(async move { let _ = server.run().await; });
        tokio::task::yield_now().await;
        pk
    });

    let mut group = c.benchmark_group("handshake_latency");
    group.measurement_time(std::time::Duration::from_secs(15));
    group.sample_size(50);

    group.bench_function("connect_send_shutdown", |b| {
        b.iter(|| {
            rt.block_on(async {
                let ckp = generate_keypair().unwrap();
                let client = IpcClient::connect(
                    uuid::Uuid::now_v7(), &path, &pub_key, ckp.as_inner(), &IpcConfig::default(), None,
                ).await.unwrap();
                client.send_frame(b"bench", std::time::Duration::from_secs(5)).await;
                client.shutdown().await;
            });
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

criterion_group!(
    benches,
    bench_bulk_e2e,
    bench_noise_roundtrip,
    bench_aes_seal,
    bench_aes_decrypt,
    bench_small_message,
    bench_bulk_sizes,
    bench_handshake_rate,
    bench_pool_cycle,
    bench_reassembly,
    bench_dispatcher,
    bench_replay_filter,
    bench_memory_guard,
    bench_socket_roundtrip,
    bench_concurrent_control,
    bench_handshake_latency,
);
criterion_main!(benches);
