//! End-to-end bulk transfer benchmark over a real Unix domain socket.
//!
//! Measures complete pipeline throughput:
//! encrypt → frame → socket write → socket read → dispatch → decrypt →
//! reassemble → accumulate
//!
//! Sizes: 1, 16, 32, 64, 256, 512, 1024, 2048, 4096 MB
//! Directions: client→server, server→client, bidirectional

use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use rekindle_node::ipc::noise_keys::generate_keypair;
use rekindle_node::ipc::noise::{client_handshake, server_handshake};
use rekindle_node::ipc::transport::PeerCredentials;
use rekindle_node::ipc::bulk::{
    self,
    cipher::BulkCipher,
    dispatcher::{BulkDispatcher, DecryptedChunk},
    encrypt::build_encrypt_pool,
    frame::MAX_CHUNK_PLAIN,
    nonce::NonceCounter,
    pool::BufferPool,
    reassembly::Reassembler,
    transfer::{send_payload, BulkTransferAccumulator},
    verify::DigestAlgorithm,
};

const MB: usize = 1_000_000;

const SIZES: &[usize] = &[
    1 * MB,
    16 * MB,
    32 * MB,
    64 * MB,
    256 * MB,
    512 * MB,
    1024 * MB,
    2048 * MB,
    4096 * MB,
];

fn sock_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bench-{}-{}-{}.sock",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

struct SocketPair {
    c_writer: tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    c_reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    s_writer: tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    s_reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    client_cipher: Arc<BulkCipher>,
    server_cipher: Arc<BulkCipher>,
}

async fn setup_pair(path: &std::path::Path) -> SocketPair {
    let listener = tokio::net::UnixListener::bind(path).unwrap();

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (client_stream, server_stream) = tokio::join!(
        tokio::net::UnixStream::connect(path),
        async { listener.accept().await.map(|(s, _)| s) },
    );
    let client_stream = client_stream.unwrap();
    let server_stream = server_stream.unwrap();

    let (c_read, c_write) = client_stream.into_split();
    let (s_read, s_write) = server_stream.into_split();

    let mut cr = tokio::io::BufReader::with_capacity(65536, c_read);
    let mut cw = tokio::io::BufWriter::with_capacity(65536, c_write);
    let mut sr = tokio::io::BufReader::with_capacity(65536, s_read);
    let mut sw = tokio::io::BufWriter::with_capacity(65536, s_write);

    let (mut ct, mut st) = tokio::join!(
        async {
            client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc)
                .await.unwrap()
        },
        async {
            server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc)
                .await.unwrap()
        },
    );

    let client_cipher = Arc::new(bulk::kdf::derive_bulk_cipher(
        &ct.take_handshake_hash().unwrap(),
    ));
    let server_cipher = Arc::new(bulk::kdf::derive_bulk_cipher(
        &st.take_handshake_hash().unwrap(),
    ));

    SocketPair { c_writer: cw, c_reader: cr, s_writer: sw, s_reader: sr, client_cipher, server_cipher }
}

async fn write_frames<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    rx: &crossbeam::channel::Receiver<Vec<u8>>,
    timeout_secs: u64,
) {
    while let Ok(frame) = rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        let lane = if frame.len() > 1 { frame[1] } else { 0x01 };
        let len = frame.len() as u32;
        writer.write_all(&[lane]).await.unwrap();
        writer.write_all(&len.to_be_bytes()).await.unwrap();
        writer.write_all(&frame).await.unwrap();
    }
    writer.flush().await.unwrap();
}

async fn read_reassemble<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    cipher: &Arc<BulkCipher>,
    encrypt_pool: &Arc<rayon::ThreadPool>,
    num_frames: usize,
    expected_size: u64,
    timeout_secs: u64,
) {
    let recv_pool = BufferPool::new();
    let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(cipher), Arc::clone(encrypt_pool), reassembly_tx, recv_pool,
    );
    let mut reassembler = Reassembler::new(1024);
    let mut accumulator = BulkTransferAccumulator::new(expected_size);

    for _ in 0..num_frames {
        let mut lane_buf = [0u8; 1];
        reader.read_exact(&mut lane_buf).await.unwrap();
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await.unwrap();
        let body_len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await.unwrap();
        dispatcher.dispatch(body).unwrap();
    }

    loop {
        match reassembly_rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
            Ok(chunk) => {
                let delivered = reassembler.process(chunk).unwrap();
                for r in &delivered {
                    accumulator.push(r);
                }
                if accumulator.is_complete() { break; }
            }
            Err(_) => panic!("bench timeout"),
        }
    }
    assert!(accumulator.is_complete());
}

fn timeout_for(size: usize) -> u64 {
    30 + (size as u64 / (100 * 1024 * 1024))
}

fn num_chunks_for(size: usize) -> usize {
    if size == 0 { 1 } else { (size + MAX_CHUNK_PLAIN - 1) / MAX_CHUNK_PLAIN }
}

// ── Client → Server ─────────────────────────────────────────────────

fn bench_client_to_server(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let encrypt_pool = build_encrypt_pool();

    let mut group = c.benchmark_group("bulk_socket_c2s");
    group.sample_size(10);

    for &size in SIZES {
        let label = format!("{} MB", size / MB);
        group.throughput(Throughput::Bytes(size as u64));
        group.measurement_time(std::time::Duration::from_secs(timeout_for(size)));

        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let num_chunks = num_chunks_for(size);
        let encrypt_pool = Arc::clone(&encrypt_pool);

        group.bench_function(&label, |b| {
            b.iter(|| {
                let payload = payload.clone();
                let encrypt_pool = Arc::clone(&encrypt_pool);
                let timeout = timeout_for(size);

                rt.block_on(async {
                    let path = sock_path(&format!("c2s-{}", size / MB));
                    let _ = std::fs::remove_file(&path);
                    let mut pair = setup_pair(&path).await;

                    let pool = BufferPool::new();
                    let nonce = Arc::new(NonceCounter::new());
                    let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

                    send_payload(
                        &encrypt_pool, &pair.client_cipher, &nonce, &pool,
                        tx, 0, &payload, DigestAlgorithm::Blake3,
                    );

                    let write_handle = tokio::spawn(async move {
                        write_frames(&mut pair.c_writer, &rx, timeout).await;
                    });

                    read_reassemble(
                        &mut pair.s_reader, &pair.server_cipher, &encrypt_pool,
                        num_chunks, size as u64, timeout,
                    ).await;

                    write_handle.await.unwrap();
                    let _ = std::fs::remove_file(&path);
                });
            });
        });
    }
    group.finish();
}

// ── Server → Client ─────────────────────────────────────────────────

fn bench_server_to_client(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let encrypt_pool = build_encrypt_pool();

    let mut group = c.benchmark_group("bulk_socket_s2c");
    group.sample_size(10);

    for &size in SIZES {
        let label = format!("{} MB", size / MB);
        group.throughput(Throughput::Bytes(size as u64));
        group.measurement_time(std::time::Duration::from_secs(timeout_for(size)));

        let payload: Vec<u8> = (0..size).map(|i| (i % 197) as u8).collect();
        let num_chunks = num_chunks_for(size);
        let encrypt_pool = Arc::clone(&encrypt_pool);

        group.bench_function(&label, |b| {
            b.iter(|| {
                let payload = payload.clone();
                let encrypt_pool = Arc::clone(&encrypt_pool);
                let timeout = timeout_for(size);

                rt.block_on(async {
                    let path = sock_path(&format!("s2c-{}", size / MB));
                    let _ = std::fs::remove_file(&path);
                    let mut pair = setup_pair(&path).await;

                    let pool = BufferPool::new();
                    let nonce = Arc::new(NonceCounter::new());
                    let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

                    send_payload(
                        &encrypt_pool, &pair.server_cipher, &nonce, &pool,
                        tx, 0, &payload, DigestAlgorithm::Blake3,
                    );

                    let write_handle = tokio::spawn(async move {
                        write_frames(&mut pair.s_writer, &rx, timeout).await;
                    });

                    read_reassemble(
                        &mut pair.c_reader, &pair.client_cipher, &encrypt_pool,
                        num_chunks, size as u64, timeout,
                    ).await;

                    write_handle.await.unwrap();
                    let _ = std::fs::remove_file(&path);
                });
            });
        });
    }
    group.finish();
}

// ── Bidirectional ───────────────────────────────────────────────────

fn bench_bidirectional(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let encrypt_pool = build_encrypt_pool();

    let mut group = c.benchmark_group("bulk_socket_bidir");
    group.sample_size(10);

    for &size in SIZES {
        let label = format!("{} MB", size / MB);
        group.throughput(Throughput::Bytes(size as u64 * 2));
        group.measurement_time(std::time::Duration::from_secs(timeout_for(size)));

        let c_payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let s_payload: Vec<u8> = (0..size).map(|i| (i % 197) as u8).collect();
        let num_chunks = num_chunks_for(size);
        let encrypt_pool = Arc::clone(&encrypt_pool);

        group.bench_function(&label, |b| {
            b.iter(|| {
                let c_payload = c_payload.clone();
                let s_payload = s_payload.clone();
                let encrypt_pool = Arc::clone(&encrypt_pool);
                let timeout = timeout_for(size);

                rt.block_on(async {
                    let path = sock_path(&format!("bidir-{}", size / MB));
                    let _ = std::fs::remove_file(&path);
                    let pair = setup_pair(&path).await;

                    let c_pool = BufferPool::new();
                    let c_nonce = Arc::new(NonceCounter::new());
                    let (c_tx, c_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
                    send_payload(
                        &encrypt_pool, &pair.client_cipher, &c_nonce, &c_pool,
                        c_tx, 0, &c_payload, DigestAlgorithm::Blake3,
                    );

                    let s_pool = BufferPool::new();
                    let s_nonce = Arc::new(NonceCounter::new());
                    let (s_tx, s_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
                    send_payload(
                        &encrypt_pool, &pair.server_cipher, &s_nonce, &s_pool,
                        s_tx, 0, &s_payload, DigestAlgorithm::Blake3,
                    );

                    let mut c_writer = pair.c_writer;
                    let mut s_writer = pair.s_writer;
                    let mut c_reader = pair.c_reader;
                    let mut s_reader = pair.s_reader;
                    let server_cipher = Arc::clone(&pair.server_cipher);
                    let client_cipher = Arc::clone(&pair.client_cipher);
                    let ep1 = Arc::clone(&encrypt_pool);
                    let ep2 = Arc::clone(&encrypt_pool);

                    let ((), (), (), ()) = tokio::join!(
                        async move { write_frames(&mut c_writer, &c_rx, timeout).await },
                        async move { write_frames(&mut s_writer, &s_rx, timeout).await },
                        async move {
                            read_reassemble(
                                &mut s_reader, &server_cipher, &ep1,
                                num_chunks, size as u64, timeout,
                            ).await
                        },
                        async move {
                            read_reassemble(
                                &mut c_reader, &client_cipher, &ep2,
                                num_chunks, size as u64, timeout,
                            ).await
                        },
                    );

                    let _ = std::fs::remove_file(&path);
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_client_to_server, bench_server_to_client, bench_bidirectional);
criterion_main!(benches);
