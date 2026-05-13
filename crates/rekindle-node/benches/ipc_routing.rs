//! Benchmarks for the IPC routing hot path.
//!
//! Measures the actual cost of each operation in the route_frame pipeline:
//! - postcard::take_from_bytes::<RoutingHeader> (partial deserialization)
//! - postcard::from_bytes::<Message<BusPayload>> (full deserialization)
//! - postcard::to_allocvec (serialization / re-encode)
//! - classify_payload (discriminant byte read)
//! - TokenBucket::try_consume (atomic rate limiting)
//! - DashMap::get (sharded connection lookup)
//! - Bytes::clone vs Bytes::copy_from_slice (zero-copy vs memcpy)
//!
//! These benchmarks produce the actual numbers for comparing the
//! pre-optimization path (full decode + re-encode) vs the post-optimization
//! path (routing header + discriminant + raw forward).

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use uuid::Uuid;

use rekindle_node::ipc::framing::{decode_frame, encode_frame};
use rekindle_node::ipc::message::{
    Message, MessageContext, RoutingHeader, SecurityLevel,
};
use rekindle_node::ipc::protocol::{BusPayload, IpcRequest};

/// Build a realistic IPC request frame (ChannelSend — the most common request type).
fn make_channel_send_frame() -> Vec<u8> {
    let ctx = MessageContext::new(Uuid::now_v7());
    let request = IpcRequest::ChannelSend {
        community: "VLD0:abc123def456:xyz789".to_string(),
        channel: "general".to_string(),
        body: "Hello, this is a test message with some realistic content length.".to_string(),
        reply_to: None,
        client_msg_id: None,
    };
    let msg = Message::new(
        &ctx,
        BusPayload::Request(request),
        SecurityLevel::Authenticated,
        Instant::now(),
    );
    encode_frame(&msg).unwrap()
}

/// Build a small Status request frame.
fn make_status_frame() -> Vec<u8> {
    let ctx = MessageContext::new(Uuid::now_v7());
    let msg = Message::new(
        &ctx,
        BusPayload::Request(IpcRequest::Status),
        SecurityLevel::Open,
        Instant::now(),
    );
    encode_frame(&msg).unwrap()
}

// ── Individual Operation Benchmarks ───────────────────────────────────

fn print_capabilities() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let caps = rekindle_node::ipc::bulk::capability::probe();
        eprintln!("\n[bench] AES-GCM: {:.2}/{:.2} GiB/s seal/open | AEGIS: {:.2} GiB/s | SHA256-mb: {:.0} MiB/s (SIMD: {}) | BLAKE3: {:.2} GiB/s",
            caps.aes_gcm_seal_gibs, caps.aes_gcm_open_gibs, caps.aegis_seal_gibs,
            caps.sha256_mb_mibs, caps.sha256_mb_simd_active, caps.blake3_gibs);
    });
}

fn bench_take_from_bytes_routing_header(c: &mut Criterion) {
    print_capabilities();
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame);

    c.bench_function("take_from_bytes::<RoutingHeader>", |b| {
        b.iter(|| {
            let (header, _remaining) =
                postcard::take_from_bytes::<RoutingHeader>(black_box(&bytes)).unwrap();
            black_box(header);
        })
    });
}

fn bench_full_decode_frame(c: &mut Criterion) {
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame);

    c.bench_function("decode_frame::<Message<BusPayload>>", |b| {
        b.iter(|| {
            let msg: Message<BusPayload> = decode_frame(black_box(&bytes)).unwrap();
            black_box(msg);
        })
    });
}

fn bench_encode_frame(c: &mut Criterion) {
    let ctx = MessageContext::new(Uuid::now_v7());
    let request = IpcRequest::ChannelSend {
        community: "VLD0:abc123def456:xyz789".to_string(),
        channel: "general".to_string(),
        body: "Hello, this is a test message with some realistic content length.".to_string(),
        reply_to: None,
        client_msg_id: None,
    };
    let msg = Message::new(
        &ctx,
        BusPayload::Request(request),
        SecurityLevel::Authenticated,
        Instant::now(),
    );

    c.bench_function("encode_frame::<Message<BusPayload>>", |b| {
        b.iter(|| {
            let encoded = encode_frame(black_box(&msg)).unwrap();
            black_box(encoded);
        })
    });
}

fn bench_classify_payload(c: &mut Criterion) {
    let frame = make_channel_send_frame();
    // Parse past the routing header to get the remaining bytes
    let (_header, remaining) =
        postcard::take_from_bytes::<RoutingHeader>(&frame).unwrap();
    let remaining = remaining.to_vec();

    c.bench_function("classify_payload (1 byte read)", |b| {
        b.iter(|| {
            let kind = black_box(remaining.first());
            black_box(kind);
        })
    });
}

fn bench_bytes_clone_vs_copy(c: &mut Criterion) {
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame.clone());

    let mut group = c.benchmark_group("bytes_forwarding");

    group.bench_function("Bytes::clone (atomic refcount)", |b| {
        b.iter(|| {
            let cloned = black_box(&bytes).clone();
            black_box(cloned);
        })
    });

    group.bench_function("Bytes::copy_from_slice (memcpy)", |b| {
        b.iter(|| {
            let copied = Bytes::copy_from_slice(black_box(&frame));
            black_box(copied);
        })
    });

    group.bench_function("Bytes::from(Vec) (ownership transfer)", |b| {
        b.iter(|| {
            let vec = frame.clone(); // clone the Vec first (this is the alloc)
            let transferred = Bytes::from(black_box(vec));
            black_box(transferred);
        })
    });

    group.finish();
}

// ── Pipeline Benchmarks ──────────────────────────────────────────────

fn bench_old_pipeline(c: &mut Criterion) {
    // Simulates the pre-optimization path:
    // decode_frame → mutate verified_sender_name → encode_frame
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame);

    c.bench_function("old_pipeline: decode + stamp + re-encode", |b| {
        b.iter(|| {
            // Full decode
            let mut msg: Message<BusPayload> = decode_frame(black_box(&bytes)).unwrap();
            // Stamp
            msg.verified_sender_name = Some(Arc::from("daemon"));
            // Re-encode
            let re_encoded = encode_frame(&msg).unwrap();
            black_box(re_encoded);
        })
    });
}

fn bench_new_pipeline(c: &mut Criterion) {
    // Simulates the post-optimization path:
    // take_from_bytes::<RoutingHeader> → classify discriminant → forward raw Bytes
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame);

    c.bench_function("new_pipeline: header + classify + forward raw", |b| {
        b.iter(|| {
            // Partial decode (routing header only)
            let (header, remaining) =
                postcard::take_from_bytes::<RoutingHeader>(black_box(&bytes)).unwrap();
            black_box(&header);

            // Classify by discriminant byte
            let _kind = remaining.first();

            // Forward raw Bytes (atomic refcount clone)
            let forwarded = bytes.clone();
            black_box(forwarded);
        })
    });
}

fn bench_pipeline_comparison(c: &mut Criterion) {
    let frame = make_channel_send_frame();
    let bytes = Bytes::from(frame);

    let mut group = c.benchmark_group("route_frame_pipeline");

    group.bench_function("old: decode+stamp+reencode", |b| {
        b.iter(|| {
            let mut msg: Message<BusPayload> = decode_frame(black_box(&bytes)).unwrap();
            msg.verified_sender_name = Some(Arc::from("daemon"));
            let re_encoded = encode_frame(&msg).unwrap();
            black_box(re_encoded);
        })
    });

    group.bench_function("new: header+classify+forward", |b| {
        b.iter(|| {
            let (header, remaining) =
                postcard::take_from_bytes::<RoutingHeader>(black_box(&bytes)).unwrap();
            black_box(&header);
            let _kind = remaining.first();
            let forwarded = bytes.clone();
            black_box(forwarded);
        })
    });

    group.finish();
}

// ── Frame Size Survey ────────────────────────────────────────────────

fn bench_frame_sizes(c: &mut Criterion) {
    // Show the actual byte sizes of typical frames for BufWriter capacity tuning
    let channel_send = make_channel_send_frame();
    let status = make_status_frame();

    println!("--- Frame sizes ---");
    println!("ChannelSend: {} bytes", channel_send.len());
    println!("Status:      {} bytes", status.len());

    // Bench decode at both sizes to show scaling
    let mut group = c.benchmark_group("decode_by_size");

    group.bench_function(format!("decode {}B (Status)", status.len()), |b| {
        b.iter(|| {
            let msg: Message<BusPayload> = decode_frame(black_box(&status)).unwrap();
            black_box(msg);
        })
    });

    group.bench_function(
        format!("decode {}B (ChannelSend)", channel_send.len()),
        |b| {
            b.iter(|| {
                let msg: Message<BusPayload> =
                    decode_frame(black_box(&channel_send)).unwrap();
                black_box(msg);
            })
        },
    );

    group.finish();
}

// ── Noise Encrypt/Decrypt Benchmark ──────────────────────────────────
//
// Measures the raw ChaCha20-Poly1305 AEAD cost via snow directly.
// No async I/O, no tokio runtime, no duplex channels.
//
// Uses the snow upstream canonical benchmark pattern (benches/benches.rs):
// - Handshake outside b.iter() (setup, not measured)
// - Paired encrypt+decrypt inside b.iter() (nonces stay synchronized)
// - Throughput::Bytes for MB/s reporting
// - Pre-allocated buffers reused across iterations
//
// What write_message measures (per snow source):
//   key_schedule (ChaCha20Poly1305::new — fresh instance per call)
//   + copy_slices(plaintext, out)
//   + encrypt_in_place_detached (ChaCha20 XOR + Poly1305 MAC)
//   + copy_slices(tag, out)
// The per-call key schedule is snow's single largest hidden cost.

/// Complete an IK handshake and return a StatelessTransportState pair.
/// Stateless transport takes explicit nonce — no auto-increment, no drift.
fn make_stateless_transport_pair(
    params: &str,
    prologue: &[u8],
) -> (snow::StatelessTransportState, snow::StatelessTransportState) {
    let server_kp = snow::Builder::new(params.parse().unwrap())
        .generate_keypair()
        .unwrap();
    let client_kp = snow::Builder::new(params.parse().unwrap())
        .generate_keypair()
        .unwrap();

    let mut initiator = snow::Builder::new(params.parse().unwrap())
        .local_private_key(&client_kp.private).unwrap()
        .remote_public_key(&server_kp.public).unwrap()
        .prologue(prologue).unwrap()
        .build_initiator()
        .unwrap();
    let mut responder = snow::Builder::new(params.parse().unwrap())
        .local_private_key(&server_kp.private).unwrap()
        .prologue(prologue).unwrap()
        .build_responder()
        .unwrap();

    let mut buf = [0u8; 256];
    let mut payload_buf = [0u8; 256];

    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload_buf).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload_buf).unwrap();

    let h_i = initiator.into_stateless_transport_mode().unwrap();
    let h_r = responder.into_stateless_transport_mode().unwrap();
    (h_i, h_r)
}

fn bench_noise_encrypt_decrypt(c: &mut Criterion) {
    use rekindle_node::ipc::noise_keys::NOISE_PARAMS;

    let channel_send_frame = make_channel_send_frame();
    let status_frame = make_status_frame();

    let mut group = c.benchmark_group("noise_crypto");

    // ── Isolated encrypt (StatelessTransportState, fixed nonce=0) ──
    //
    // StatelessTransportState::write_message(&self, nonce, payload, out)
    // takes &self and explicit nonce. Nonce never auto-increments.
    // Same nonce every iteration — measures pure encrypt cost including
    // per-call ChaCha20Poly1305::new() key schedule.
    {
        let (h_i, _h_r) = make_stateless_transport_pair(NOISE_PARAMS, b"BENCH");
        let msg_size = channel_send_frame.len();
        let mut enc_buf = vec![0u8; msg_size + 16]; // plaintext + TAGLEN

        group.throughput(criterion::Throughput::Bytes(msg_size as u64));
        group.bench_function(
            format!("encrypt_only {}B (ChannelSend)", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_i
                        .write_message(0, black_box(&channel_send_frame), &mut enc_buf)
                        .unwrap();
                    black_box(len);
                })
            },
        );
    }

    {
        let (h_i, _h_r) = make_stateless_transport_pair(NOISE_PARAMS, b"BENCH");
        let msg_size = status_frame.len();
        let mut enc_buf = vec![0u8; msg_size + 16];

        group.throughput(criterion::Throughput::Bytes(msg_size as u64));
        group.bench_function(
            format!("encrypt_only {}B (Status)", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_i
                        .write_message(0, black_box(&status_frame), &mut enc_buf)
                        .unwrap();
                    black_box(len);
                })
            },
        );
    }

    // ── Isolated decrypt (StatelessTransportState, fixed nonce=0) ──
    //
    // Pre-encrypt once with nonce=0 outside b.iter(). Decrypt with
    // nonce=0 on every iteration. No nonce drift — pure decrypt cost.
    {
        let (h_i, h_r) = make_stateless_transport_pair(NOISE_PARAMS, b"BENCH");
        let msg_size = channel_send_frame.len();
        let mut enc_buf = vec![0u8; msg_size + 16];
        let mut dec_buf = vec![0u8; msg_size];

        // Pre-encrypt once
        let ct_len = h_i
            .write_message(0, &channel_send_frame, &mut enc_buf)
            .unwrap();
        let ciphertext = enc_buf[..ct_len].to_vec();

        group.throughput(criterion::Throughput::Bytes(msg_size as u64));
        group.bench_function(
            format!("decrypt_only {}B (ChannelSend)", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_r
                        .read_message(0, black_box(&ciphertext), &mut dec_buf)
                        .unwrap();
                    black_box(len);
                })
            },
        );
    }

    // ── Paired encrypt+decrypt (canonical snow pattern) ───────────
    //
    // Uses TransportState (stateful, auto-nonce) matching the actual
    // runtime path. Nonces stay synchronized: each b.iter() does one
    // encrypt (h_i.n++) then one decrypt (h_r.n++).
    {
        let server_kp = snow::Builder::new(NOISE_PARAMS.parse().unwrap())
            .generate_keypair()
            .unwrap();
        let client_kp = snow::Builder::new(NOISE_PARAMS.parse().unwrap())
            .generate_keypair()
            .unwrap();
        let mut initiator = snow::Builder::new(NOISE_PARAMS.parse().unwrap())
            .local_private_key(&client_kp.private).unwrap()
            .remote_public_key(&server_kp.public).unwrap()
            .prologue(b"BENCH").unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = snow::Builder::new(NOISE_PARAMS.parse().unwrap())
            .local_private_key(&server_kp.private).unwrap()
            .prologue(b"BENCH").unwrap()
            .build_responder()
            .unwrap();
        let mut buf = [0u8; 256];
        let mut payload_buf = [0u8; 256];
        let len = initiator.write_message(&[], &mut buf).unwrap();
        responder.read_message(&buf[..len], &mut payload_buf).unwrap();
        let len = responder.write_message(&[], &mut buf).unwrap();
        initiator.read_message(&buf[..len], &mut payload_buf).unwrap();
        let mut h_i = initiator.into_transport_mode().unwrap();
        let mut h_r = responder.into_transport_mode().unwrap();

        let msg_size = channel_send_frame.len();
        let mut buffer_msg = vec![0u8; msg_size * 2];
        let mut buffer_out = vec![0u8; msg_size * 2];
        buffer_msg[..msg_size].copy_from_slice(&channel_send_frame);

        group.throughput(criterion::Throughput::Bytes((msg_size * 2) as u64));
        group.bench_function(
            format!("roundtrip {}B (ChannelSend)", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_i
                        .write_message(black_box(&buffer_msg[..msg_size]), &mut buffer_out)
                        .unwrap();
                    let _ = h_r
                        .read_message(black_box(&buffer_out[..len]), &mut buffer_msg)
                        .unwrap();
                })
            },
        );
    }

    group.finish();
}

// ── I/O Benchmarks ───────────────────────────────────────────────────
//
// Two tiers:
// 1. Duplex (in-memory): measures framing + crypto + tokio overhead
//    without kernel syscall cost. Isolates our code from the OS.
// 2. Real Unix socket: measures end-to-end including writev/readv syscalls,
//    kernel buffer copies, and context switch latency.
//
// Both use a complete Noise IK handshake → transport pair, writing
// encrypted frames through the full write_encrypted_frame / read_encrypted_frame
// path including BufWriter batching and length-prefix framing.

/// Create a Noise transport pair over a tokio::io::duplex channel.
/// Returns (client_transport, server_transport, client_read, client_write,
///          server_read, server_write).
/// Create a Noise transport pair over duplex with BufWriter/BufReader.
///
/// The handshake runs through the same BufWriter/BufReader that the
/// transport uses. This is critical: if the handshake runs on raw
/// split halves and then BufWriter/BufReader are added after, residual
/// handshake bytes in the duplex cause the transport to misparse frames.
async fn make_noise_duplex_pair() -> (
    rekindle_node::ipc::NoiseTransport,
    rekindle_node::ipc::NoiseTransport,
    tokio::io::BufWriter<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    tokio::io::BufWriter<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
) {
    use rekindle_node::ipc::noise::{client_handshake, server_handshake};
    use rekindle_node::ipc::noise_keys::generate_keypair;
    use rekindle_node::ipc::transport::PeerCredentials;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(cs);
    let (sr, sw) = tokio::io::split(ss);

    // Handshake through BufReader/BufWriter — same wrappers the transport uses.
    let mut cr = tokio::io::BufReader::with_capacity(8192, cr);
    let mut cw = tokio::io::BufWriter::with_capacity(8192, cw);
    let mut sr = tokio::io::BufReader::with_capacity(8192, sr);
    let mut sw = tokio::io::BufWriter::with_capacity(8192, sw);

    let (ct, st) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc),
    );

    (ct.unwrap(), st.unwrap(), cw, cr, sw, sr)
}

// ── I/O Benchmark Helpers ────────────────────────────────────────────

/// Build a multi-thread tokio runtime for I/O benchmarks.
fn io_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// Create a Noise-encrypted Unix socket pair for benchmarks.
/// Returns (client_transport, server_transport, client_writer, server_reader).
/// Handshake runs through the same BufWriter/BufReader the transport uses.
async fn make_noise_unix_pair(
    sock_path: &std::path::Path,
) -> (
    rekindle_node::ipc::NoiseTransport,
    rekindle_node::ipc::NoiseTransport,
    tokio::io::BufWriter<tokio::net::unix::OwnedWriteHalf>,
    tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
) {
    use rekindle_node::ipc::noise::{client_handshake, server_handshake};
    use rekindle_node::ipc::noise_keys::generate_keypair;
    use rekindle_node::ipc::transport::PeerCredentials;

    let listener = tokio::net::UnixListener::bind(sock_path).unwrap();

    let (client_stream, server_stream) = tokio::join!(
        tokio::net::UnixStream::connect(sock_path),
        async { listener.accept().await.map(|(s, _)| s) },
    );
    let client_stream = client_stream.unwrap();
    let server_stream = server_stream.unwrap();

    let (cr, cw) = client_stream.into_split();
    let (sr, sw) = server_stream.into_split();

    let mut cr = tokio::io::BufReader::with_capacity(8192, cr);
    let mut cw = tokio::io::BufWriter::with_capacity(8192, cw);
    let mut sr = tokio::io::BufReader::with_capacity(8192, sr);
    let mut sw = tokio::io::BufWriter::with_capacity(8192, sw);

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (ct, st) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc),
    );

    // Drop the server write + client read halves — we only benchmark
    // the client→server direction. Dropping sw/cr is fine since the
    // halves are independent after split.
    drop(sw);
    drop(cr);

    (ct.unwrap(), st.unwrap(), cw, sr)
}

// ── Duplex Roundtrip ────────────────────────────────────────────────
//
// Single-frame write + flush + read over in-memory duplex.
// Measures: encrypt + framing + BufWriter memcpy + duplex transfer
//           + BufReader fill + framing + decrypt + BytesMut split.
// Throughput: 1 frame × msg_size per iteration.

fn bench_io_duplex_roundtrip(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();

    let mut group = c.benchmark_group("io_duplex_roundtrip");
    group.throughput(criterion::Throughput::Bytes(msg_size as u64));
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(
        format!("roundtrip {}B", msg_size),
        |b| {
            let (mut ct, mut st, cw, _cr, _sw, sr) =
                rt.block_on(make_noise_duplex_pair());
            let mut writer = cw;
            let mut reader = sr;

            b.iter(|| {
                rt.block_on(async {
                    ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                        .await
                        .unwrap();
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                    black_box(&frame);
                })
            })
        },
    );

    group.finish();
}

// ── Duplex Batch Write ──────────────────────────────────────────────
//
// 32 frames written without explicit per-frame flush, then one flush,
// then 32 sequential reads. Handshake is performed once outside b.iter().
//
// 32 × 177 B (4-byte chunk-count frame + 4-byte length prefix + 165 B
// ciphertext per frame) = 5664 B total — fits in the 256 KiB duplex
// buffer without backpressure. Sequential read after write is safe.
//
// Measures: amortized encrypt + BufWriter batching (auto-flush at 8 KiB
// boundary, ~46 frames, so 32 frames may or may not trigger auto-flush)
// + single explicit flush + decrypt.
// Throughput: 32 frames × msg_size per iteration.

fn bench_io_duplex_batch(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();
    const BATCH: usize = 32;

    let mut group = c.benchmark_group("io_duplex_batch");
    group.throughput(criterion::Throughput::Bytes((BATCH * msg_size) as u64));
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(
        format!("batch_{}x{}B", BATCH, msg_size),
        |b| {
            let (mut ct, mut st, cw, _cr, _sw, sr) =
                rt.block_on(make_noise_duplex_pair());
            let mut writer = cw;
            let mut reader = sr;

            b.iter(|| {
                rt.block_on(async {
                    // Write 32 frames — BufWriter accumulates, may auto-flush
                    // at its 8 KiB boundary.
                    for _ in 0..BATCH {
                        ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                            .await
                            .unwrap();
                    }
                    // Single explicit flush pushes any remaining buffered data.
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    // Read all 32 frames sequentially.
                    for _ in 0..BATCH {
                        let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                        black_box(&frame);
                    }
                })
            })
        },
    );

    group.finish();
}

// ── Duplex Amortized Roundtrip ──────────────────────────────────────
//
// 1000 frames in a single block_on call. NO explicit per-frame flush —
// BufWriter auto-flushes at its 8 KiB boundary (~46 frames per flush),
// amortizing syscall cost across ~22 flushes instead of 1000.
//
// 1000 × 177 B = 177 KiB. Duplex buffer is 256 KiB. All frames fit
// without backpressure — write all, flush once, read all sequentially.
//
// Measures: true amortized per-frame cost under BufWriter batching,
// eliminating both per-iteration block_on overhead and per-frame flush
// overhead. This is the closest approximation to production throughput
// through the Noise transport layer.
// Throughput: 1000 frames × msg_size per iteration.

fn bench_io_duplex_amortized(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();
    const N: usize = 1000;

    let mut group = c.benchmark_group("io_duplex_amortized");
    group.throughput(criterion::Throughput::Bytes((N * msg_size) as u64));
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(
        format!("amortized_{}x{}B", N, msg_size),
        |b| {
            let (mut ct, mut st, cw, _cr, _sw, sr) =
                rt.block_on(make_noise_duplex_pair());
            let mut writer = cw;
            let mut reader = sr;

            b.iter(|| {
                rt.block_on(async {
                    // Write all N frames. BufWriter auto-flushes at 8 KiB.
                    for _ in 0..N {
                        ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                            .await
                            .unwrap();
                    }
                    // One final flush for any residual buffered data.
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    // Read all N frames sequentially.
                    for _ in 0..N {
                        let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                        black_box(&frame);
                    }
                })
            })
        },
    );

    group.finish();
}

// ── Unix Socket Roundtrip ───────────────────────────────────────────
//
// Single-frame write + flush + read over a real Unix domain socket.
// Measures the full end-to-end path including kernel writev/readv
// syscalls, kernel socket buffer copies, and epoll wakeup latency.

fn bench_io_unix_socket_roundtrip(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();

    let mut group = c.benchmark_group("io_unix_socket_roundtrip");
    group.throughput(criterion::Throughput::Bytes(msg_size as u64));
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(
        format!("roundtrip {}B", msg_size),
        |b| {
            let sock_path = std::env::temp_dir().join(format!(
                "rekindle-bench-rt-{}.sock",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&sock_path);

            let (mut ct, mut st, mut writer, mut reader) =
                rt.block_on(make_noise_unix_pair(&sock_path));

            b.iter(|| {
                rt.block_on(async {
                    ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                        .await
                        .unwrap();
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                    black_box(&frame);
                })
            });

            let _ = std::fs::remove_file(&sock_path);
        },
    );

    group.finish();
}

// ── Unix Socket Batch Write ─────────────────────────────────────────
//
// 32 frames written, single flush, 32 sequential reads — over a real
// Unix domain socket. Same pattern as duplex batch but exercises the
// full kernel I/O path: writev, epoll, readv, context switches.
//
// The kernel socket buffer (default SO_SNDBUF ~212 KiB on most Linux
// configs) comfortably holds 32 × 177 B = 5664 B. No backpressure.

fn bench_io_unix_socket_batch(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();
    const BATCH: usize = 32;

    let mut group = c.benchmark_group("io_unix_socket_batch");
    group.measurement_time(std::time::Duration::from_secs(10));
    group.throughput(criterion::Throughput::Bytes((BATCH * msg_size) as u64));

    group.bench_function(
        format!("batch_{}x{}B", BATCH, msg_size),
        |b| {
            let sock_path = std::env::temp_dir().join(format!(
                "rekindle-bench-batch-{}.sock",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&sock_path);

            let (mut ct, mut st, mut writer, mut reader) =
                rt.block_on(make_noise_unix_pair(&sock_path));

            b.iter(|| {
                rt.block_on(async {
                    for _ in 0..BATCH {
                        ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                            .await
                            .unwrap();
                    }
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    for _ in 0..BATCH {
                        let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                        black_box(&frame);
                    }
                })
            });

            let _ = std::fs::remove_file(&sock_path);
        },
    );

    group.finish();
}

// ── Unix Socket Amortized ───────────────────────────────────────────
//
// 1000 frames in a single block_on, no per-frame flush, over a real
// Unix socket. BufWriter auto-flushes at 8 KiB. The kernel socket
// buffer may fill mid-write if the reader doesn't drain — but since
// we write all then read all, and the kernel buffer is ~212 KiB while
// total payload is 177 KiB, we fit without deadlock.
//
// If SO_SNDBUF is smaller than expected, this benchmark will show
// elevated latency from BufWriter blocking on a full socket buffer.
// That's diagnostic — it reveals the kernel buffer ceiling.

fn bench_io_unix_socket_amortized(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();
    const N: usize = 1000;

    let mut group = c.benchmark_group("io_unix_socket_amortized");
    group.throughput(criterion::Throughput::Bytes((N * msg_size) as u64));
    group.measurement_time(std::time::Duration::from_secs(10));

    group.bench_function(
        format!("amortized_{}x{}B", N, msg_size),
        |b| {
            let sock_path = std::env::temp_dir().join(format!(
                "rekindle-bench-amort-{}.sock",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&sock_path);

            let (mut ct, mut st, mut writer, mut reader) =
                rt.block_on(make_noise_unix_pair(&sock_path));

            b.iter(|| {
                rt.block_on(async {
                    for _ in 0..N {
                        ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                            .await
                            .unwrap();
                    }
                    tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();

                    for _ in 0..N {
                        let frame = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
                        black_box(&frame);
                    }
                })
            });

            let _ = std::fs::remove_file(&sock_path);
        },
    );

    group.finish();
}

// ── Full-Duplex Wire Speed ───────────────────────────────────────
//
// Writer and reader run as concurrent tokio tasks on the same duplex
// stream. The writer pushes N frames without explicit per-frame flush
// (BufWriter auto-flushes at 8 KiB). The reader pulls frames as fast
// as they arrive. Measures sustained crypto pipeline throughput
// without synchronization overhead.
//
// This is the true wire speed ceiling of the Noise transport layer.

fn bench_io_duplex_fullduplex(c: &mut Criterion) {
    let rt = io_rt();
    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();
    const N: usize = 10_000;

    let mut group = c.benchmark_group("io_duplex_fullduplex");
    group.throughput(criterion::Throughput::Bytes((N * msg_size) as u64));
    group.measurement_time(std::time::Duration::from_secs(20));

    group.bench_function(
        format!("fullduplex_{N}x{msg_size}B"),
        |b| {
            let (mut ct, mut st, cw, _cr, _sw, sr) =
                rt.block_on(make_noise_duplex_pair());
            let mut writer = cw;
            let mut reader = sr;

            b.iter(|| {
                rt.block_on(async {
                    // Writer and reader run as concurrent futures via join!.
                    // No ownership transfer — both borrow from the outer scope.
                    // join! polls both on the same task, interleaving through
                    // the duplex buffer.
                    let write_fut = async {
                        for _ in 0..N {
                            ct.writer.write_encrypted_frame(&mut writer, black_box(&channel_send_frame))
                                .await
                                .unwrap();
                        }
                        tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();
                    };

                    let read_fut = async {
                        for _ in 0..N {
                            let _frame = st.reader.read_encrypted_frame(&mut reader)
                                .await
                                .unwrap();
                            black_box(&_frame);
                        }
                    };

                    tokio::join!(write_fut, read_fut);
                })
            })
        },
    );

    group.finish();
}

// ── Noise Resolver Comparison ──────────────────────────────────────
//
// Measures decrypt throughput with the aws-lc resolver (zero-alloc
// open_separate_gather) vs the default ring resolver baseline.
// The aws-lc resolver is now the default — this benchmark proves the
// improvement is measurable.

fn bench_noise_resolver_decrypt(c: &mut Criterion) {
    use rekindle_node::ipc::noise_keys::NOISE_PARAMS;

    let channel_send_frame = make_channel_send_frame();
    let msg_size = channel_send_frame.len();

    let mut group = c.benchmark_group("noise_resolver_decrypt");
    group.throughput(criterion::Throughput::Bytes(msg_size as u64));
    group.measurement_time(std::time::Duration::from_secs(15));

    // aws-lc resolver (current default — zero-alloc decrypt)
    {
        let (h_i, h_r) = make_stateless_transport_pair(NOISE_PARAMS, b"RESOLVER");
        let mut enc_buf = vec![0u8; msg_size + 16];
        let mut dec_buf = vec![0u8; msg_size];

        let ct_len = h_i.write_message(0, &channel_send_frame, &mut enc_buf).unwrap();
        let ciphertext = enc_buf[..ct_len].to_vec();

        group.bench_function(
            format!("aws_lc_decrypt {}B", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_r
                        .read_message(0, black_box(&ciphertext), &mut dec_buf)
                        .unwrap();
                    black_box(len);
                })
            },
        );
    }

    // aws-lc resolver encrypt (for symmetry comparison)
    {
        let (h_i, _h_r) = make_stateless_transport_pair(NOISE_PARAMS, b"RESOLVER");
        let mut enc_buf = vec![0u8; msg_size + 16];

        group.bench_function(
            format!("aws_lc_encrypt {}B", msg_size),
            |b| {
                b.iter(|| {
                    let len = h_i
                        .write_message(0, black_box(&channel_send_frame), &mut enc_buf)
                        .unwrap();
                    black_box(len);
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_take_from_bytes_routing_header,
    bench_full_decode_frame,
    bench_encode_frame,
    bench_classify_payload,
    bench_bytes_clone_vs_copy,
    bench_old_pipeline,
    bench_new_pipeline,
    bench_pipeline_comparison,
    bench_frame_sizes,
    bench_noise_encrypt_decrypt,
    bench_noise_resolver_decrypt,
    bench_io_duplex_roundtrip,
    bench_io_duplex_batch,
    bench_io_duplex_amortized,
    bench_io_duplex_fullduplex,
    bench_io_unix_socket_roundtrip,
    bench_io_unix_socket_batch,
    bench_io_unix_socket_amortized,
);
criterion_main!(benches);
