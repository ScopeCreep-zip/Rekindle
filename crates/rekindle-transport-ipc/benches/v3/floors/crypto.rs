//! Crypto primitive irreducible floors: per-frame floor (seal_into + MAC +
//! envelope, zero copy), copy tax ratio (seal vs seal_into).
//!
//! Report structure:
//! - per_frame_floor: line — function=aegis128l/aes256gcm, param=size (log)
//!   Throughput::Bytes(size) so as_number() returns distinct values per size.
//! - copy_tax:        line — function=seal_alloc/seal_inplace, param=size (log)
//!
//! SamplingMode::Auto — tight crypto loops where Linear regression gives
//! slope = per-iteration cost. No iter_custom lifecycle overhead to flatten.

use std::sync::Arc;

use criterion::{
    AxisScale, BenchmarkId, Criterion, PlotConfiguration, Throughput,
};
use rekindle_transport_ipc::calibrate::{OscId, Profile, CalibratedSession};

pub fn register(c: &mut Criterion, session: &mut CalibratedSession) {
    per_frame_floor(c, session);
    copy_tax(c, session);
}

// ── Per-frame irreducible floor ─────────────────────────────────────

fn per_frame_floor(c: &mut Criterion, session: &mut CalibratedSession) {
    use rekindle_transport_ipc::v3::codec::aead::FrameCipher;
    use rekindle_transport_ipc::v3::codec::envelope::{self, EnvelopeInfo};
    use rekindle_transport_ipc::v3::codec::header::{self, StreamHeaderInfo};
    use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
    use rekindle_transport_ipc::v3::wire::constants::{DIRECTION_ID_D2L, WIRE_VERSION};
    use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
    use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;
    use rekindle_transport_ipc::v3::wire::lane::Lane;

    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher_aegis = FrameCipher::new(
        Arc::new(rekindle_aead::aegis128l::Aegis128LKey::new(
            &keys.stream_d2l[..16].try_into().unwrap(),
        )),
        DIRECTION_ID_D2L,
    );
    let cipher_gcm = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).unwrap();

    let mut group = c.benchmark_group("per_frame_floor");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536] {
        let mag = super::fmt_mag(size);
        let plaintext_buf: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let wire_len = 32 + 32 + size + 16;
        let size_str = format!("{size}");

        for (algo_name, cipher) in &[("aegis128l", &cipher_aegis), ("aes256gcm", &cipher_gcm)] {
            let osc = format!(
                "osc:lat/crypto.seal+crypto.mac?size={mag}&variant={algo_name}&alloc=none"
            );
            let id = OscId::parse(&osc, Profile::L1Conditions)
                .unwrap_or_else(|e| panic!("malformed OSC: crypto.seal+mac {algo_name} {mag}: {e}"));

            if let Err(skip) = session.check_deps(&id) {
                tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
                continue;
            }

            let mut wire_buf = vec![0u8; wire_len];
            let env_info = EnvelopeInfo {
                wire_version: WIRE_VERSION, lane: Lane::Data,
                flags: 0, body_len: (32 + size + 16) as u32, session_seq: 0,
            };
            let header_info = StreamHeaderInfo {
                frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
                stream_id: 0, header_flags: 0, chunk_index: 0, nonce: 0,
            };

            group.throughput(Throughput::Bytes(size as u64));
            group.bench_function(BenchmarkId::new(*algo_name, &size_str), |b| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        let env_bytes = envelope::build_envelope(&env_info, &keys.envelope_d2l);
                        let hdr_bytes = header::build_header(&header_info, &keys.header_d2l);
                        wire_buf[..32].copy_from_slice(&env_bytes);
                        wire_buf[32..64].copy_from_slice(&hdr_bytes);
                        cipher.seal_into(0, &env_bytes, Some(&hdr_bytes),
                            &plaintext_buf, &mut wire_buf[64..], 0);
                        std::hint::black_box(&wire_buf);
                    }
                    start.elapsed()
                })
            });
            session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                Some(("per_frame_floor", algo_name, &size_str)));
        }
    }

    group.finish();
}

// ── Copy tax ratio (seal vs seal_into) ──────────────────────────────

fn copy_tax(c: &mut Criterion, session: &mut CalibratedSession) {
    use rekindle_transport_ipc::v3::codec::aead::FrameCipher;
    use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
    use rekindle_transport_ipc::v3::wire::constants::DIRECTION_ID_D2L;

    let keys = derive_all_keys(&[0xBE; 32]);
    let cipher = FrameCipher::new(
        Arc::new(rekindle_aead::aegis128l::Aegis128LKey::new(
            &keys.stream_d2l[..16].try_into().unwrap(),
        )),
        DIRECTION_ID_D2L,
    );
    let env = [0x11u8; 32];
    let hdr = [0x22u8; 32];

    let mut group = c.benchmark_group("copy_tax");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    for &size in &[64, 1024, 65536, 1024 * 1024] {
        let mag = super::fmt_mag(size);
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut out_buf = vec![0u8; size + 16];
        let size_str = format!("{size}");

        // seal (allocating)
        {
            let osc = format!("osc:bw/crypto.seal?size={mag}&variant=aegis128l&alloc=fresh");
            let id = OscId::parse(&osc, Profile::L1Conditions)
                .unwrap_or_else(|e| panic!("malformed OSC: crypto.seal alloc {mag}: {e}"));
            if session.check_deps(&id).is_ok() {
                group.throughput(Throughput::Bytes(size as u64));
                group.bench_function(BenchmarkId::new("seal_alloc", &size_str), |b| {
                    b.iter_custom(|iters| {
                        let start = std::time::Instant::now();
                        for _ in 0..iters {
                            let ct = cipher.seal(0, &env, Some(&hdr), &data);
                            std::hint::black_box(&ct);
                        }
                        start.elapsed()
                    })
                });
                session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                    Some(("copy_tax", "seal_alloc", &size_str)));
            }
        }

        // seal_into (in-place)
        {
            let osc = format!("osc:bw/crypto.seal?size={mag}&variant=aegis128l&alloc=none");
            let id = OscId::parse(&osc, Profile::L1Conditions)
                .unwrap_or_else(|e| panic!("malformed OSC: crypto.seal inplace {mag}: {e}"));
            if session.check_deps(&id).is_ok() {
                group.throughput(Throughput::Bytes(size as u64));
                group.bench_function(BenchmarkId::new("seal_inplace", &size_str), |b| {
                    b.iter_custom(|iters| {
                        let start = std::time::Instant::now();
                        for _ in 0..iters {
                            cipher.seal_into(0, &env, Some(&hdr), &data, &mut out_buf, 0);
                            std::hint::black_box(&out_buf);
                        }
                        start.elapsed()
                    })
                });
                session.record_with_coords(&id, 0.0, 0.0, 0.0, 20, "criterion.pending_backfill",
                    Some(("copy_tax", "seal_inplace", &size_str)));
            }
        }
    }

    group.finish();
}
