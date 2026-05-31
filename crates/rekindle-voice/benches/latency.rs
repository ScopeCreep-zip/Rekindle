//! Architecture §32 Phase 7 Week 26 — voice latency benchmark.
//!
//! The spec target (line 4147) is "<100ms mouth-to-ear". A true
//! mouth-to-ear measurement requires physical audio loopback with a
//! microphone and speaker (NIST's `mouth2ear` MATLAB harness is the
//! reference) and is impossible to reproduce in `cargo bench`. What we
//! CAN measure in-process is every component of the pipeline that adds
//! algorithmic or buffering latency, and assert that their sum plus
//! the documented network-side budget stays under 100ms.
//!
//! ## Measurement strategy (per the implementation plan in
//! `.claude/plans` discussion):
//!
//! - **Per-component criterion benches** measure wall-clock latency
//!   per call for each pipeline stage (`opus_encode_20ms`,
//!   `opus_decode_20ms`, `jitter_push_pop`, `mixer_4_sources`).
//! - **End-to-end loopback** runs an entire encode → packetize →
//!   jitter → decode → mix cycle in-process and reports total wall
//!   clock per iteration. This catches interaction costs (cache
//!   eviction across stages) that per-component benches miss.
//! - **Budget assertion** is a separate test
//!   (`tests/latency_budget.rs` — TODO once the bench targets stabilise)
//!   that reads criterion's `target/criterion/*/estimates.json` and
//!   asserts the sum of P95s stays under the documented budget.
//!
//! Run with: `cargo bench -p rekindle-voice --bench latency`
//!
//! Sources:
//! - Architecture §32 Phase 7 Week 26 (line 4147).
//! - NIST IR 8206 §6 — mouth-to-ear measurement methodology.
//! - Mumble #3502 — VoIP latency profile reference.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rekindle_voice::codec::OpusCodec;
use rekindle_voice::jitter::JitterBuffer;
use rekindle_voice::mixer::AudioMixer;
use rekindle_voice::transport::VoicePacket;

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 1;
/// 20ms frame at 48kHz mono = 960 samples (matches the production
/// configuration in `voice_config_for_group_size`).
const FRAME_SAMPLES: usize = 960;

/// Generate a 20ms PCM frame of synthetic speech-like content (a
/// 440 Hz sine wave). `cast_precision_loss` is a non-issue at these
/// magnitudes: `i ≤ 960` and `SAMPLE_RATE = 48_000` both fit
/// losslessly in `f32`'s 23-bit mantissa (max precise integer ≈ 16M).
#[allow(clippy::cast_precision_loss)]
fn synth_frame() -> Vec<f32> {
    let two_pi_freq = 2.0 * std::f32::consts::PI * 440.0;
    let inv_sample_rate = 1.0_f32 / SAMPLE_RATE as f32;
    (0..FRAME_SAMPLES)
        .map(|i| (two_pi_freq * (i as f32) * inv_sample_rate).sin() * 0.5)
        .collect()
}

fn bench_opus_encode_20ms(c: &mut Criterion) {
    let mut codec = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("opus init");
    let frame = synth_frame();
    c.benchmark_group("opus_encode_20ms")
        .throughput(Throughput::Elements(1))
        .bench_function("encode", |b| {
            b.iter(|| {
                let _ = codec.encode(&frame).expect("encode");
            });
        });
}

fn bench_opus_decode_20ms(c: &mut Criterion) {
    let mut codec = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("opus init");
    let frame = synth_frame();
    let encoded = codec.encode(&frame).expect("encode");
    c.benchmark_group("opus_decode_20ms")
        .throughput(Throughput::Elements(1))
        .bench_function("decode", |b| {
            b.iter(|| {
                let _ = codec.decode(&encoded).expect("decode");
            });
        });
}

fn bench_jitter_push_pop(c: &mut Criterion) {
    // Target depth 60ms (3 frames) — middle of the production
    // dynamic range. Each iteration pushes a fresh ordered packet
    // and immediately pops, exercising the BTreeMap insert + remove
    // path that dominates jitter cost.
    c.benchmark_group("jitter_push_pop_60ms_target")
        .throughput(Throughput::Elements(1))
        .bench_function("push_pop", |b| {
            let mut jb = JitterBuffer::new(60);
            let mut seq = 0u32;
            // Pre-fill so pop has something to return on each call.
            for _ in 0..3 {
                jb.push(make_packet(seq));
                seq += 1;
            }
            b.iter(|| {
                jb.push(make_packet(seq));
                seq = seq.wrapping_add(1);
                let _ = jb.pop();
            });
        });
}

fn bench_mixer(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixer");
    let frame = synth_frame();
    for sources in [1, 2, 4, 8usize] {
        group.throughput(Throughput::Elements(sources as u64));
        group.bench_with_input(BenchmarkId::from_parameter(sources), &sources, |b, &n| {
            let mixer = AudioMixer::new(CHANNELS);
            let frames: Vec<Vec<f32>> = (0..n).map(|_| frame.clone()).collect();
            let pseudonyms: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            b.iter(|| {
                let streams: Vec<(&str, &[f32])> = pseudonyms
                    .iter()
                    .zip(frames.iter())
                    .map(|(p, f)| (p.as_str(), f.as_slice()))
                    .collect();
                let _ = mixer.mix(&streams);
            });
        });
    }
}

/// End-to-end loopback: encode → packetize → jitter buffer
/// (push+pop) → decode → mixer (single source). Every algorithmic step
/// the production pipeline performs except network transport. The
/// result is the per-frame compute cost; mouth-to-ear adds the fixed
/// algorithmic delay (Opus VoIP at 48kHz: ~6.5ms encode + ~6.5ms
/// decode lookahead) plus jitter target depth plus capture/playback
/// buffers plus network RTT.
fn bench_e2e_loopback(c: &mut Criterion) {
    let mut encoder = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("encoder init");
    let mut decoder = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("decoder init");
    let mut jb = JitterBuffer::new(60);
    let mixer = AudioMixer::new(CHANNELS);
    let frame = synth_frame();

    // Pre-warm jitter so the first iteration's `pop` returns Some.
    for seq in 0..3 {
        let encoded = encoder.encode(&frame).expect("warmup encode");
        jb.push(VoicePacket {
            sender_key: vec![1u8; 32],
            sequence: seq,
            timestamp: u64::from(seq) * 20,
            audio_data: encoded.data,
            signature: Vec::new(),
        });
    }
    let mut seq = 3u32;

    c.benchmark_group("e2e_loopback_20ms_frame")
        .throughput(Throughput::Elements(1))
        .bench_function("loopback", |b| {
            b.iter(|| {
                let encoded = encoder.encode(&frame).expect("encode");
                jb.push(VoicePacket {
                    sender_key: vec![1u8; 32],
                    sequence: seq,
                    timestamp: u64::from(seq) * 20,
                    audio_data: encoded.data,
                    signature: Vec::new(),
                });
                seq = seq.wrapping_add(1);
                if let Some(packet) = jb.pop() {
                    let dec_frame = rekindle_voice::codec::EncodedFrame {
                        data: packet.audio_data,
                        timestamp: packet.timestamp,
                        sequence: packet.sequence,
                    };
                    let decoded = decoder.decode(&dec_frame).expect("decode");
                    let _ = mixer.mix(&[("p0", &decoded.samples)]);
                }
            });
        });
}

fn make_packet(seq: u32) -> VoicePacket {
    VoicePacket {
        sender_key: vec![1u8; 32],
        sequence: seq,
        timestamp: u64::from(seq) * 20,
        // 80 bytes is a typical 32 kbps 20 ms Opus frame size.
        audio_data: vec![0xAB; 80],
        signature: Vec::new(),
    }
}

criterion_group!(
    benches,
    bench_opus_encode_20ms,
    bench_opus_decode_20ms,
    bench_jitter_push_pop,
    bench_mixer,
    bench_e2e_loopback,
);
criterion_main!(benches);
