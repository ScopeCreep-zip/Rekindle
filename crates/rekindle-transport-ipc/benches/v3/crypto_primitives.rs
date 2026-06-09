//! v3 primitive benchmarks: EMAC, HeaderMAC, AEAD seal/open, audit chain,
//! reassembler, pool slab, handshake rate. Establishes the theoretical
//! performance ceiling for each subsystem in isolation.
//!
//! Each measurement is identified by an `osc:` identifier, recorded with
//! harness coordinates for backfill, and emitted as structured JSONL via
//! `finalize()`.
//!
//! Run: `cargo bench -p rekindle-transport-ipc --features bench-harness --bench v3_crypto_primitives`

use std::time::Duration;
use criterion::{BenchmarkId, Criterion, Throughput};

use rekindle_transport_ipc::calibrate::{
    CalibratedSession, OscId, Profile,
    init_tracing, finalize, fmt_mag,
};
use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput};
use rekindle_transport_ipc::v3::bulk::pool::BufferPool;
use rekindle_transport_ipc::v3::codec::aead::FrameCipher;
use rekindle_transport_ipc::v3::codec::envelope::{self, EnvelopeInfo};
use rekindle_transport_ipc::v3::codec::header::{self, StreamHeaderInfo};
use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
use rekindle_transport_ipc::v3::io::decode::FrameDecoder;
use rekindle_transport_ipc::v3::io::encode::FrameEncoder;
use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;
use rekindle_transport_ipc::v3::wire::constants::{DIRECTION_ID_D2L, ENVELOPE_LEN, WIRE_VERSION};
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;
use rekindle_transport_ipc::v3::wire::lane::Lane;

fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

const SIZES: &[(usize, &str)] = &[
    (64, "64B"), (1024, "1KiB"), (65536, "64KiB"),
    (1024 * 1024, "1MiB"),
    (16 * 1024 * 1024 - 48, "~16MiB"),
];

/// Record an OSC measurement into the session with harness coordinates.
fn osc_record(
    session: &mut CalibratedSession,
    osc_str: &str,
    group: &str,
    function: &str,
    param: &str,
    sample_count: u32,
) {
    let id = OscId::parse(osc_str, Profile::L0Core)
        .unwrap_or_else(|e| panic!("malformed OSC id '{osc_str}': {e}"));
    session.record_with_coords(
        &id, 0.0, 0.0, 0.0, sample_count, "criterion.pending_backfill",
        Some((group, function, param)),
    );
}

// ── EMAC ────────────────────────────────────────────────────────

fn emac(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let env = envelope::build_envelope(&EnvelopeInfo {
        wire_version: WIRE_VERSION, lane: Lane::Control,
        flags: 0, body_len: 100, session_seq: 42,
    }, &keys.envelope_d2l);

    let mut group = c.benchmark_group("emac");
    group.throughput(Throughput::Elements(1));
    group.bench_function("verify", |b| {
        b.iter(|| envelope::parse_envelope(&env, &keys.envelope_d2l).unwrap())
    });
    osc_record(session, "osc:lat/crypto.emac?variant=verify", "emac", "verify", "", 30);
    group.finish();
}

// ── HeaderMAC ───────────────────────────────────────────────────

fn header_mac(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let info = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
        stream_id: 0, header_flags: 0, chunk_index: 0, nonce: 42,
    };
    let hdr = header::build_header(&info, &keys.header_d2l);

    let mut group = c.benchmark_group("header_mac");
    group.throughput(Throughput::Elements(1));
    group.bench_function("build", |b| {
        b.iter(|| header::build_header(&info, &keys.header_d2l))
    });
    osc_record(session, "osc:lat/crypto.header_mac?variant=build", "header_mac", "build", "", 30);
    group.bench_function("verify", |b| {
        b.iter(|| header::parse_header(&hdr, &keys.header_d2l).unwrap())
    });
    osc_record(session, "osc:lat/crypto.header_mac?variant=verify", "header_mac", "verify", "", 30);
    group.finish();
}

// ── AEAD seal ───────────────────────────────────────────────────

fn aead_seal(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];

    let ciphers: Vec<(&str, FrameCipher)> = vec![
        ("aes256gcm", FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap()),
        ("aegis128l", FrameCipher::new(
            std::sync::Arc::new(rekindle_aead::aegis128l::Aegis128LKey::new(&keys.stream_d2l[..16].try_into().unwrap())),
            DIRECTION_ID_D2L,
        )),
        ("aegis128x2", FrameCipher::aegis128x2(&keys.stream_d2l, DIRECTION_ID_D2L)),
    ];

    let mut group = c.benchmark_group("aead_seal");
    for (algo_name, cipher) in &ciphers {
        for &(size, size_label) in SIZES {
            let data = payload(size);
            group.throughput(Throughput::Bytes(size as u64));
            group.bench_with_input(
                BenchmarkId::new(*algo_name, size_label), &data,
                |b, data| b.iter(|| cipher.seal(42, &env, Some(&hdr), data)),
            );
            let mag = fmt_mag(size);
            osc_record(session,
                &format!("osc:bw/crypto.seal?size={mag}&variant={algo_name}&alloc=fresh"),
                "aead_seal", algo_name, size_label, 30,
            );
        }
    }
    group.finish();
}

// ── AEAD open ───────────────────────────────────────────────────

fn aead_open(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];

    let ciphers: Vec<(&str, FrameCipher)> = vec![
        ("aes256gcm", FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap()),
        ("aegis128l", FrameCipher::new(
            std::sync::Arc::new(rekindle_aead::aegis128l::Aegis128LKey::new(&keys.stream_d2l[..16].try_into().unwrap())),
            DIRECTION_ID_D2L,
        )),
        ("aegis128x2", FrameCipher::aegis128x2(&keys.stream_d2l, DIRECTION_ID_D2L)),
    ];

    let mut group = c.benchmark_group("aead_open");
    for (algo_name, cipher) in &ciphers {
        for &(size, size_label) in SIZES {
            let data = payload(size);
            let ct = cipher.seal(42, &env, Some(&hdr), &data);
            group.throughput(Throughput::Bytes(size as u64));
            group.bench_with_input(
                BenchmarkId::new(*algo_name, size_label), &ct,
                |b, ct| b.iter(|| cipher.open(42, &env, Some(&hdr), ct).unwrap()),
            );
            let mag = fmt_mag(size);
            osc_record(session,
                &format!("osc:bw/crypto.open?size={mag}&variant={algo_name}"),
                "aead_open", algo_name, size_label, 30,
            );
        }
    }
    group.finish();
}

// ── AEAD seal_into ──────────────────────────────────────────────

fn aead_seal_into(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];

    let ciphers: Vec<(&str, FrameCipher)> = vec![
        ("aes256gcm", FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap()),
        ("aegis128l", FrameCipher::new(
            std::sync::Arc::new(rekindle_aead::aegis128l::Aegis128LKey::new(&keys.stream_d2l[..16].try_into().unwrap())),
            DIRECTION_ID_D2L,
        )),
        ("aegis128x2", FrameCipher::aegis128x2(&keys.stream_d2l, DIRECTION_ID_D2L)),
    ];

    let mut group = c.benchmark_group("aead_seal_into");
    for (algo_name, cipher) in &ciphers {
        for &(size, size_label) in SIZES {
            let data = payload(size);
            let wire_len = 32 + 32 + size + 16;
            group.throughput(Throughput::Bytes(size as u64));
            group.bench_with_input(
                BenchmarkId::new(*algo_name, size_label), &data,
                |b, data| {
                    let mut buf = vec![0u8; wire_len];
                    b.iter(|| {
                        cipher.seal_into(42, &env, Some(&hdr), data, &mut buf[64..], 0);
                    })
                },
            );
            let mag = fmt_mag(size);
            osc_record(session,
                &format!("osc:bw/crypto.seal?size={mag}&variant={algo_name}&alloc=none"),
                "aead_seal_into", algo_name, size_label, 30,
            );
        }
    }
    group.finish();
}

// ── Audit chain ─────────────────────────────────────────────────

fn audit_chain_link(c: &mut Criterion, session: &mut CalibratedSession) {
    let input = LinkInput {
        session_seq: 0, envelope_hash: [0x11; 32],
        header_hash: [0x22; 32], ciphertext_hash: [0x33; 32],
    };
    let mut group = c.benchmark_group("audit_chain");
    group.throughput(Throughput::Elements(1));
    group.bench_function("link_advance", |b| {
        let mut chain = AuditChain::new([0xAA; 32], [0xBB; 32]);
        b.iter(|| chain.advance(input))
    });
    osc_record(session, "osc:lat/audit.link", "audit_chain", "link_advance", "", 30);
    group.finish();
}

fn audit_chain_sustained(c: &mut Criterion, session: &mut CalibratedSession) {
    let input = LinkInput {
        session_seq: 0, envelope_hash: [0x11; 32],
        header_hash: [0x22; 32], ciphertext_hash: [0x33; 32],
    };

    let mut group = c.benchmark_group("audit_chain_sustained");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("10k_links", |b| {
        b.iter(|| {
            let mut chain = AuditChain::new([0xAA; 32], [0xBB; 32]);
            for _ in 0..10_000 {
                chain.advance(input);
            }
            std::hint::black_box(chain.current_link());
        })
    });
    osc_record(session, "osc:ops/audit.link?contention=none&variant=sustained", "audit_chain_sustained", "10k_links", "", 30);
    group.finish();
}

// ── EMAC density ────────────────────────────────────────────────

fn emac_200k_density(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let env = envelope::build_envelope(&EnvelopeInfo {
        wire_version: WIRE_VERSION, lane: Lane::Control,
        flags: 0, body_len: 100, session_seq: 42,
    }, &keys.envelope_d2l);

    let mut group = c.benchmark_group("emac_density");
    group.throughput(Throughput::Elements(200_000));
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    group.bench_function("200k_burst", |b| {
        b.iter(|| {
            for _ in 0..200_000 {
                std::hint::black_box(
                    envelope::parse_envelope(&env, &keys.envelope_d2l).unwrap()
                );
            }
        })
    });
    osc_record(session, "osc:ops/crypto.emac?contention=none&variant=200k_burst", "emac_density", "200k_burst", "", 10);
    group.finish();
}

// ── Reassembler ─────────────────────────────────────────────────

fn reassembler_in_order(c: &mut Criterion, session: &mut CalibratedSession) {
    let chunk = payload(1024);
    let digest = *blake3::hash(&chunk).as_bytes();
    let mut group = c.benchmark_group("reassembler");
    group.throughput(Throughput::Bytes(1024));
    group.bench_function("in_order_1KiB", |b| {
        b.iter(|| {
            let mut r = Reassembler::new(256);
            for i in 0..100u32 {
                let delivered = r.insert_with_digest(i, PlaintextBuf::Owned(chunk.clone()), digest);
                drop(delivered);
            }
            std::hint::black_box(r.next_expected());
        })
    });
    osc_record(session, "osc:bw/reassemble.insert?size=1k&order=sequential", "reassembler", "in_order_1KiB", "", 30);
    group.finish();
}

fn reassembler_out_of_order(c: &mut Criterion, session: &mut CalibratedSession) {
    let chunk = payload(1024);
    let digest = *blake3::hash(&chunk).as_bytes();
    let mut group = c.benchmark_group("reassembler_ooo");
    group.throughput(Throughput::Bytes(1024));
    group.bench_function("50pct_ooo_1KiB", |b| {
        b.iter(|| {
            let mut r = Reassembler::new(256);
            for i in (0..100u32).step_by(2) {
                let delivered = r.insert_with_digest(i, PlaintextBuf::Owned(chunk.clone()), digest);
                drop(delivered);
            }
            for i in (1..100u32).step_by(2) {
                let delivered = r.insert_with_digest(i, PlaintextBuf::Owned(chunk.clone()), digest);
                drop(delivered);
            }
            std::hint::black_box(r.next_expected());
        })
    });
    osc_record(session, "osc:bw/reassemble.insert?size=1k&order=ooo_50pct", "reassembler_ooo", "50pct_ooo_1KiB", "", 30);
    group.finish();
}

fn reassembler_16mib_chunks(c: &mut Criterion, session: &mut CalibratedSession) {
    let chunk = payload(16 * 1024 * 1024 - 48);
    let digest = *blake3::hash(&chunk).as_bytes();

    let mut group = c.benchmark_group("reassembler_16mib");
    group.throughput(Throughput::Bytes(4 * (16 * 1024 * 1024 - 48)));
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    group.bench_function("4_chunks_in_order", |b| {
        b.iter(|| {
            let mut r = Reassembler::new(256);
            for i in 0..4u32 {
                let delivered = r.insert_with_digest(i, PlaintextBuf::Owned(chunk.clone()), digest);
                drop(delivered);
            }
            std::hint::black_box(r.next_expected());
        })
    });
    osc_record(session, "osc:bw/reassemble.insert?size=16m&order=sequential", "reassembler_16mib", "4_chunks_in_order", "", 10);
    group.finish();
}

// ── Pool ────────────────────────────────────────────────────────

fn pool_slab(c: &mut Criterion, session: &mut CalibratedSession) {
    let pool = BufferPool::with_capacity(64, 1024);
    let mut group = c.benchmark_group("pool");
    group.throughput(Throughput::Elements(1));
    group.bench_function("acquire_release", |b| {
        b.iter(|| {
            let slab = pool.try_acquire().unwrap();
            pool.release(slab);
            pool.reclaim();
        })
    });
    osc_record(session, "osc:lat/pool.acquire?contention=none&variant=slab", "pool", "acquire_release", "", 30);
    group.finish();
}

fn pool_contention(c: &mut Criterion, session: &mut CalibratedSession) {
    let pool = BufferPool::with_capacity(32, 1024);
    let mut group = c.benchmark_group("pool_contention");
    group.throughput(Throughput::Elements(1));

    for &threads in &[2usize, 4, 8] {
        let param = format!("{threads}_threads");
        group.bench_function(BenchmarkId::from_parameter(&param), |b| {
            b.iter_custom(|iters| {
                let barrier = std::sync::Arc::new(std::sync::Barrier::new(threads));
                let pool = pool.clone();
                let handles: Vec<_> = (0..threads).map(|_| {
                    let b = barrier.clone();
                    let p = pool.clone();
                    let per_thread = iters as usize / threads;
                    std::thread::spawn(move || {
                        b.wait();
                        let start = std::time::Instant::now();
                        for _ in 0..per_thread {
                            if let Some(slab) = p.try_acquire() {
                                p.release(slab);
                                p.reclaim();
                            }
                        }
                        start.elapsed()
                    })
                }).collect();
                handles.into_iter()
                    .map(|h| h.join().unwrap())
                    .max()
                    .unwrap_or_default()
            })
        });
        osc_record(session,
            &format!("osc:lat/pool.acquire?contention={threads}t&variant=workload"),
            "pool_contention", &param, "", 30,
        );
    }
    group.finish();
}

// ── Encode/Decode pipeline ──────────────────────────────────────

fn encode_pipeline(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();
    let encoder = FrameEncoder::new(keys.envelope_d2l, keys.header_d2l, cipher);

    let mut group = c.benchmark_group("encode_pipeline");
    for &(size, label) in SIZES {
        let data = payload(size);
        let frame = if size <= 60000 {
            OutboundFrame::Channel {
                kind: rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind::Ping,
                payload: data,
            }
        } else {
            OutboundFrame::Data {
                stream_id: 0, kind: StreamKind::Payload,
                chunk_index: 0, payload: data,
            }
        };
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &frame, |b, f| {
            b.iter(|| encoder.encode_with_audit(f))
        });
        let mag = fmt_mag(size);
        osc_record(session,
            &format!("osc:bw/pipeline.encode?size={mag}"),
            "encode_pipeline", label, "", 30,
        );
    }
    group.finish();
}

fn decode_pipeline(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let send_cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();
    let recv_cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();
    let encoder = FrameEncoder::new(keys.envelope_d2l, keys.header_d2l, send_cipher);
    let decoder = FrameDecoder::new(keys.envelope_d2l, keys.header_d2l, recv_cipher);

    let mut group = c.benchmark_group("decode_pipeline");
    for &(size, label) in SIZES {
        let data = payload(size);
        let frame = OutboundFrame::Data {
            stream_id: 0, kind: StreamKind::Payload,
            chunk_index: 0, payload: data,
        };
        let ewa = encoder.encode_with_audit(&frame);
        let wire = ewa.encoded.into_wire_bytes();
        let env_bytes: [u8; ENVELOPE_LEN] = wire[..ENVELOPE_LEN].try_into().unwrap();
        let body = wire[ENVELOPE_LEN..].to_vec();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &(&env_bytes, &body), |b, (env, body)| {
            b.iter(|| {
                let info = envelope::parse_envelope(env, &keys.envelope_d2l).unwrap();
                decoder.decode_body(env, &info, body).unwrap()
            })
        });
        let mag = fmt_mag(size);
        osc_record(session,
            &format!("osc:bw/pipeline.decode?size={mag}"),
            "decode_pipeline", label, "", 30,
        );
    }
    group.finish();
}

fn encode_bulk_production(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();
    let encoder = FrameEncoder::new(keys.envelope_d2l, keys.header_d2l, cipher);

    let data = payload(16 * 1024 * 1024 - 48);
    let frame = OutboundFrame::Data {
        stream_id: 0, kind: StreamKind::Payload,
        chunk_index: 0, payload: data,
    };

    let mut group = c.benchmark_group("encode_bulk_production");
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    group.bench_function("encode_16MiB", |b| {
        b.iter(|| encoder.encode_with_audit(&frame))
    });
    osc_record(session, "osc:bw/pipeline.encode?size=16m&variant=bulk", "encode_bulk_production", "encode_16MiB", "", 10);
    group.finish();
}

// ── Handshake ───────────────────────────────────────────────────

fn handshake_rate(c: &mut Criterion, session: &mut CalibratedSession) {
    use rekindle_transport_ipc::v3::crypto::noise::{generate_keypair, build_initiator, build_responder};

    let mut group = c.benchmark_group("handshake");
    group.throughput(Throughput::Elements(1));
    group.bench_function("noise_ik", |b| {
        b.iter(|| {
            let skp = generate_keypair().unwrap();
            let ckp = generate_keypair().unwrap();
            let mut init = build_initiator(&ckp.private, &skp.public, b"BENCH").unwrap();
            let mut resp = build_responder(&skp.private, b"BENCH").unwrap();
            let mut buf = [0u8; 256];
            let mut pay = [0u8; 256];
            let len = init.write_message(&[], &mut buf).unwrap();
            resp.read_message(&buf[..len], &mut pay).unwrap();
            let len = resp.write_message(&[], &mut buf).unwrap();
            init.read_message(&buf[..len], &mut pay).unwrap();
            let _ = init.into_stateless_transport_mode().unwrap();
            let _ = resp.into_stateless_transport_mode().unwrap();
        })
    });
    osc_record(session, "osc:lat/crypto.handshake?variant=noise_ik", "handshake", "noise_ik", "", 30);
    group.finish();
}

// ── Parallel encrypt/decrypt/encode ─────────────────────────────

fn parallel_encrypt(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = std::sync::Arc::new(
        FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap()
    );
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];
    let chunk = payload(16 * 1024 * 1024 - 48);

    let mut group = c.benchmark_group("parallel_encrypt");
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    let chunk_arc: std::sync::Arc<[u8]> = chunk.into();

    for &workers in &[1usize, 2, 4, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers).build().unwrap();
        let total_bytes = chunk_arc.len() * workers;

        let bufs: Vec<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> = (0..workers)
            .map(|_| std::sync::Arc::new(std::sync::Mutex::new(vec![0u8; chunk_arc.len() + 16])))
            .collect();

        group.throughput(Throughput::Bytes(total_bytes as u64));
        let param = format!("{workers}_workers");
        group.bench_with_input(
            BenchmarkId::from_parameter(&param),
            &workers, |b, &n| {
                b.iter(|| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    for i in 0..n {
                        let c = cipher.clone();
                        let data = chunk_arc.clone();
                        let buf = bufs[i].clone();
                        let tx = tx.clone();
                        pool.spawn(move || {
                            let mut buf = buf.lock().unwrap();
                            c.seal_into(42, &env, Some(&hdr), &data, &mut buf, 0);
                            tx.send(buf.len()).unwrap();
                        });
                    }
                    drop(tx);
                    let count: usize = rx.iter().count();
                    assert_eq!(count, n);
                })
            },
        );
        osc_record(session,
            &format!("osc:bw/crypto.seal?size=16m&variant=aes256gcm&alloc=none&workers={workers}w"),
            "parallel_encrypt", &param, "", 10,
        );
    }
    group.finish();
}

fn parallel_decrypt(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = std::sync::Arc::new(
        FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap()
    );
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];
    let chunk = payload(16 * 1024 * 1024 - 48);
    let ciphertext: std::sync::Arc<[u8]> = cipher.seal(42, &env, Some(&hdr), &chunk).into();

    let mut group = c.benchmark_group("parallel_decrypt");
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    for &workers in &[1usize, 2, 4, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers).build().unwrap();
        let total_bytes = chunk.len() * workers;

        group.throughput(Throughput::Bytes(total_bytes as u64));
        let param = format!("{workers}_workers");
        group.bench_with_input(
            BenchmarkId::from_parameter(&param),
            &workers, |b, &n| {
                b.iter(|| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    for _ in 0..n {
                        let c = cipher.clone();
                        let ct = ciphertext.clone();
                        let tx = tx.clone();
                        pool.spawn(move || {
                            let pt = c.open(42, &env, Some(&hdr), &ct).unwrap();
                            tx.send(pt.len()).unwrap();
                        });
                    }
                    drop(tx);
                    let count: usize = rx.iter().count();
                    assert_eq!(count, n);
                })
            },
        );
        osc_record(session,
            &format!("osc:bw/crypto.open?size=16m&variant=aes256gcm&workers={workers}w"),
            "parallel_decrypt", &param, "", 10,
        );
    }
    group.finish();
}

fn parallel_encode(c: &mut Criterion, session: &mut CalibratedSession) {
    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();
    let encoder = std::sync::Arc::new(
        FrameEncoder::new(keys.envelope_d2l, keys.header_d2l, cipher)
    );
    let chunk: std::sync::Arc<[u8]> = payload(16 * 1024 * 1024 - 48).into();

    let mut group = c.benchmark_group("parallel_encode");
    group.measurement_time(Duration::from_secs(60));
    group.sample_size(10);

    for &workers in &[1usize, 2, 4, 8] {
        let rayon_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers).build().unwrap();
        let total_bytes = chunk.len() * workers;

        group.throughput(Throughput::Bytes(total_bytes as u64));
        let param = format!("{workers}_workers");
        group.bench_with_input(
            BenchmarkId::from_parameter(&param),
            &workers, |b, &n| {
                b.iter(|| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    for i in 0..n {
                        let enc = encoder.clone();
                        let data: Vec<u8> = (*chunk).to_vec();
                        let tx = tx.clone();
                        rayon_pool.spawn(move || {
                            let frame = OutboundFrame::Data {
                                stream_id: i as u8,
                                kind: StreamKind::Payload,
                                chunk_index: 0,
                                payload: data,
                            };
                            let ewa = enc.encode_with_audit(&frame);
                            tx.send(ewa.link_input.session_seq).unwrap();
                        });
                    }
                    drop(tx);
                    let count: usize = rx.iter().count();
                    assert_eq!(count, n);
                })
            },
        );
        osc_record(session,
            &format!("osc:bw/pipeline.encode?size=16m&variant=parallel&workers={workers}w"),
            "parallel_encode", &param, "", 10,
        );
    }
    group.finish();
}

// ── Main ────────────────────────────────────────────────────────

fn main() {
    init_tracing();
    tracing::info!("crypto_primitives: starting");

    let mut criterion = Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .sample_size(30)
        .nresamples(100_000)
        .configure_from_args();

    let mut session = CalibratedSession::new(Profile::L0Core);
    tracing::info!(
        profile = ?session.profile(),
        substrate = %session.substrate().canonical(),
        "crypto_primitives: session created"
    );

    emac(&mut criterion, &mut session);
    header_mac(&mut criterion, &mut session);
    aead_seal(&mut criterion, &mut session);
    aead_open(&mut criterion, &mut session);
    aead_seal_into(&mut criterion, &mut session);
    audit_chain_link(&mut criterion, &mut session);
    audit_chain_sustained(&mut criterion, &mut session);
    emac_200k_density(&mut criterion, &mut session);
    reassembler_in_order(&mut criterion, &mut session);
    reassembler_out_of_order(&mut criterion, &mut session);
    reassembler_16mib_chunks(&mut criterion, &mut session);
    pool_slab(&mut criterion, &mut session);
    pool_contention(&mut criterion, &mut session);
    encode_pipeline(&mut criterion, &mut session);
    decode_pipeline(&mut criterion, &mut session);
    encode_bulk_production(&mut criterion, &mut session);
    handshake_rate(&mut criterion, &mut session);
    parallel_encrypt(&mut criterion, &mut session);
    parallel_decrypt(&mut criterion, &mut session);
    parallel_encode(&mut criterion, &mut session);

    finalize(&mut session, &mut criterion, "crypto");
}
