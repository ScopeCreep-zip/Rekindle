//! Integration tests: bulk transfer over a real Unix domain socket.
//!
//! These tests prove data moves end-to-end through the complete stack:
//! Noise IK handshake → bulk cipher derivation → encrypt → frame →
//! socket write → socket read → dispatch → decrypt → reassemble →
//! accumulate → verify payload.
//!
//! Test matrix (9 sizes × 3 directions = 27 tests):
//! - Serial client→server: 1, 16, 32, 64, 256, 512, 1024, 2048, 4096 MB
//! - Serial server→client: 1, 16, 32, 64, 256, 512, 1024, 2048, 4096 MB
//! - Bidirectional simultaneous: 1, 16, 32, 64, 256, 512, 1024, 2048, 4096 MB

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

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

fn test_socket_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bulk-{}-{}-{}.sock",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn make_payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

async fn handshake_pair(
    sock_path: &std::path::Path,
) -> (
    rekindle_node::ipc::noise::NoiseWriter,
    tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    rekindle_node::ipc::noise::NoiseReader,
    tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    Arc<BulkCipher>,
    rekindle_node::ipc::noise::NoiseWriter,
    tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    rekindle_node::ipc::noise::NoiseReader,
    tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    Arc<BulkCipher>,
) {
    let listener = UnixListener::bind(sock_path).unwrap();

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (client_stream, server_stream) = tokio::join!(
        tokio::net::UnixStream::connect(sock_path),
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
                .await
                .unwrap()
        },
        async {
            server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc)
                .await
                .unwrap()
        },
    );

    let client_cipher = Arc::new(bulk::kdf::derive_bulk_cipher(
        &ct.take_handshake_hash().unwrap(),
    ));
    let server_cipher = Arc::new(bulk::kdf::derive_bulk_cipher(
        &st.take_handshake_hash().unwrap(),
    ));

    (
        ct.writer, cw, ct.reader, cr, client_cipher,
        st.writer, sw, st.reader, sr, server_cipher,
    )
}

async fn write_bulk_frames<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    rx: &crossbeam::channel::Receiver<Vec<u8>>,
    timeout: std::time::Duration,
) -> usize {
    let mut count = 0;
    while let Ok(frame) = rx.recv_timeout(timeout) {
        let lane = if frame.len() > 1 { frame[1] } else { 0x01 };
        let len = frame.len() as u32;
        writer.write_all(&[lane]).await.unwrap();
        writer.write_all(&len.to_be_bytes()).await.unwrap();
        writer.write_all(&frame).await.unwrap();
        count += 1;
    }
    writer.flush().await.unwrap();
    count
}

async fn read_and_reassemble<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    cipher: &Arc<BulkCipher>,
    num_frames: usize,
    timeout_secs: u64,
) -> Vec<u8> {
    let encrypt_pool = build_encrypt_pool();
    let recv_pool = BufferPool::new();
    let (reassembly_tx, reassembly_rx) = crossbeam::channel::bounded::<DecryptedChunk>(256);
    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(cipher),
        Arc::clone(&encrypt_pool),
        reassembly_tx,
        recv_pool,
    );
    let mut reassembler = Reassembler::new(1024);
    let mut accumulator = BulkTransferAccumulator::new(0);

    for _ in 0..num_frames {
        let mut lane_buf = [0u8; 1];
        reader.read_exact(&mut lane_buf).await.unwrap();
        assert!(
            (0x01..=0x03).contains(&lane_buf[0]),
            "expected bulk lane, got 0x{:02x}",
            lane_buf[0]
        );

        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await.unwrap();
        let body_len = u32::from_be_bytes(len_buf) as usize;

        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await.unwrap();

        dispatcher.dispatch(body).unwrap();
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        match reassembly_rx.recv_timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
        ) {
            Ok(chunk) => {
                let delivered = reassembler.process(chunk).unwrap();
                for r in &delivered {
                    if let Some(complete) = accumulator.push(r) {
                        return complete;
                    }
                }
            }
            Err(_) => panic!(
                "timed out waiting for reassembly — got {}/{} chunks",
                accumulator.chunks_received(),
                num_frames,
            ),
        }
    }
}

fn timeout_for(size: usize) -> u64 {
    30 + (size as u64 / (100 * 1024 * 1024))
}

fn num_chunks_for(size: usize) -> usize {
    if size == 0 { 1 } else { (size + MAX_CHUNK_PLAIN - 1) / MAX_CHUNK_PLAIN }
}

async fn run_client_to_server(size: usize, label: &str) {
    let sock_path = test_socket_path(label);
    let _ = std::fs::remove_file(&sock_path);

    let (
        _c_noise_w, mut c_writer, _c_noise_r, _c_reader, client_cipher,
        _s_noise_w, _s_writer, _s_noise_r, mut s_reader, server_cipher,
    ) = handshake_pair(&sock_path).await;

    let payload = make_payload(size);
    let num_chunks = num_chunks_for(size);
    let timeout_secs = timeout_for(size);

    let encrypt_pool = build_encrypt_pool();
    let buffer_pool = BufferPool::new();
    let nonce_ctr = Arc::new(NonceCounter::new());
    let (frame_tx, frame_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

    send_payload(
        &encrypt_pool, &client_cipher, &nonce_ctr, &buffer_pool,
        frame_tx, 0, &payload, DigestAlgorithm::Blake3,
    );

    let write_handle = tokio::spawn(async move {
        write_bulk_frames(&mut c_writer, &frame_rx, std::time::Duration::from_secs(timeout_secs)).await
    });

    let received = read_and_reassemble(&mut s_reader, &server_cipher, num_chunks, timeout_secs).await;

    let frames_written = write_handle.await.unwrap();
    assert_eq!(frames_written, num_chunks, "{label}: frame count mismatch");
    assert_eq!(received.len(), payload.len(), "{label}: payload size mismatch");
    assert_eq!(received, payload, "{label}: payload content mismatch");

    let _ = std::fs::remove_file(&sock_path);
}

async fn run_server_to_client(size: usize, label: &str) {
    let sock_path = test_socket_path(label);
    let _ = std::fs::remove_file(&sock_path);

    let (
        _c_noise_w, _c_writer, _c_noise_r, mut c_reader, client_cipher,
        _s_noise_w, mut s_writer, _s_noise_r, _s_reader, server_cipher,
    ) = handshake_pair(&sock_path).await;

    let payload = make_payload(size);
    let num_chunks = num_chunks_for(size);
    let timeout_secs = timeout_for(size);

    let encrypt_pool = build_encrypt_pool();
    let buffer_pool = BufferPool::new();
    let nonce_ctr = Arc::new(NonceCounter::new());
    let (frame_tx, frame_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);

    send_payload(
        &encrypt_pool, &server_cipher, &nonce_ctr, &buffer_pool,
        frame_tx, 0, &payload, DigestAlgorithm::Blake3,
    );

    let write_handle = tokio::spawn(async move {
        write_bulk_frames(&mut s_writer, &frame_rx, std::time::Duration::from_secs(timeout_secs)).await
    });

    let received = read_and_reassemble(&mut c_reader, &client_cipher, num_chunks, timeout_secs).await;

    let frames_written = write_handle.await.unwrap();
    assert_eq!(frames_written, num_chunks, "{label}: frame count mismatch");
    assert_eq!(received.len(), payload.len(), "{label}: payload size mismatch");
    assert_eq!(received, payload, "{label}: payload content mismatch");

    let _ = std::fs::remove_file(&sock_path);
}

async fn run_bidirectional(size: usize, label: &str) {
    let sock_path = test_socket_path(label);
    let _ = std::fs::remove_file(&sock_path);

    let (
        _c_noise_w, mut c_writer, _c_noise_r, mut c_reader, client_cipher,
        _s_noise_w, mut s_writer, _s_noise_r, mut s_reader, server_cipher,
    ) = handshake_pair(&sock_path).await;

    let client_payload = make_payload(size);
    let server_payload: Vec<u8> = (0..size).map(|i| (i % 197) as u8).collect();
    let num_chunks = num_chunks_for(size);
    let timeout_secs = timeout_for(size);
    let timeout_dur = std::time::Duration::from_secs(timeout_secs);

    let encrypt_pool = build_encrypt_pool();

    let c_pool = BufferPool::new();
    let c_nonce = Arc::new(NonceCounter::new());
    let (c_frame_tx, c_frame_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
    send_payload(
        &encrypt_pool, &client_cipher, &c_nonce, &c_pool,
        c_frame_tx, 0, &client_payload, DigestAlgorithm::Blake3,
    );

    let s_pool = BufferPool::new();
    let s_nonce = Arc::new(NonceCounter::new());
    let (s_frame_tx, s_frame_rx) = crossbeam::channel::bounded::<Vec<u8>>(256);
    send_payload(
        &encrypt_pool, &server_cipher, &s_nonce, &s_pool,
        s_frame_tx, 0, &server_payload, DigestAlgorithm::Blake3,
    );

    let server_cipher_recv = Arc::clone(&server_cipher);
    let client_cipher_recv = Arc::clone(&client_cipher);

    let (c_write_result, s_write_result, s_recv_result, c_recv_result) = tokio::join!(
        async move { write_bulk_frames(&mut c_writer, &c_frame_rx, timeout_dur).await },
        async move { write_bulk_frames(&mut s_writer, &s_frame_rx, timeout_dur).await },
        async move { read_and_reassemble(&mut s_reader, &server_cipher_recv, num_chunks, timeout_secs).await },
        async move { read_and_reassemble(&mut c_reader, &client_cipher_recv, num_chunks, timeout_secs).await },
    );

    assert_eq!(c_write_result, num_chunks, "{label}: client write frame count");
    assert_eq!(s_write_result, num_chunks, "{label}: server write frame count");
    assert_eq!(s_recv_result.len(), client_payload.len(), "{label}: server recv size");
    assert_eq!(s_recv_result, client_payload, "{label}: server recv content");
    assert_eq!(c_recv_result.len(), server_payload.len(), "{label}: client recv size");
    assert_eq!(c_recv_result, server_payload, "{label}: client recv content");

    let _ = std::fs::remove_file(&sock_path);
}

const MB: usize = 1_000_000;

// ── Client → Server ─────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_1mb() { run_client_to_server(1 * MB, "c2s-1mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_16mb() { run_client_to_server(16 * MB, "c2s-16mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_32mb() { run_client_to_server(32 * MB, "c2s-32mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_64mb() { run_client_to_server(64 * MB, "c2s-64mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_256mb() { run_client_to_server(256 * MB, "c2s-256mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_512mb() { run_client_to_server(512 * MB, "c2s-512mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_1024mb() { run_client_to_server(1024 * MB, "c2s-1024mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_2048mb() { run_client_to_server(2048 * MB, "c2s-2048mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn c2s_4096mb() { run_client_to_server(4096 * MB, "c2s-4096mb").await; }

// ── Server → Client ─────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_1mb() { run_server_to_client(1 * MB, "s2c-1mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_16mb() { run_server_to_client(16 * MB, "s2c-16mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_32mb() { run_server_to_client(32 * MB, "s2c-32mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_64mb() { run_server_to_client(64 * MB, "s2c-64mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_256mb() { run_server_to_client(256 * MB, "s2c-256mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_512mb() { run_server_to_client(512 * MB, "s2c-512mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_1024mb() { run_server_to_client(1024 * MB, "s2c-1024mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_2048mb() { run_server_to_client(2048 * MB, "s2c-2048mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2c_4096mb() { run_server_to_client(4096 * MB, "s2c-4096mb").await; }

// ── Bidirectional ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_1mb() { run_bidirectional(1 * MB, "bidir-1mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_16mb() { run_bidirectional(16 * MB, "bidir-16mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_32mb() { run_bidirectional(32 * MB, "bidir-32mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_64mb() { run_bidirectional(64 * MB, "bidir-64mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_256mb() { run_bidirectional(256 * MB, "bidir-256mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_512mb() { run_bidirectional(512 * MB, "bidir-512mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_1024mb() { run_bidirectional(1024 * MB, "bidir-1024mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_2048mb() { run_bidirectional(2048 * MB, "bidir-2048mb").await; }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bidir_4096mb() { run_bidirectional(4096 * MB, "bidir-4096mb").await; }
