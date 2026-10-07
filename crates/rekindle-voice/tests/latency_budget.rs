//! Voice latency budget regression guard, baselined to ITU-T G.114
//! interactive-voice limits (one-way ≤150ms = good, ≤400ms = acceptable).
//!
//! The original "<100ms mouth-to-ear" target (Architecture §32 Phase 7
//! W26 line 4147) assumed a 0-hop `SafetySelection::Unsafe` voice route
//! (~5ms LAN network leg). Voice now routes over a 3-hop Tor-class
//! `SafetySelection::Safe` route so the sender stays anonymous, which
//! moves the network leg into the G.114 interactive band. The budget
//! ceiling is re-baselined accordingly — a deliberate privacy/latency
//! tradeoff, not a loosened test.
//!
//! True mouth-to-ear measurement requires physical loopback (NIST IR
//! 8206 §6 / `mouth2ear` MATLAB harness). What we CAN guard in CI is
//! the algorithmic + buffering budget the in-process pipeline adds on
//! top of the network round-trip. This test runs each stage in a tight
//! loop, measures wall-clock P95 directly, and asserts the sum plus
//! the documented network-side budget stays below the G.114 ceiling.
//!
//! Run with: `cargo test -p rekindle-voice --release --test latency_budget`
//! (the `--release` is important — debug-mode Opus / RNNoise are
//! 5-10x slower and would inflate the per-stage P95 spuriously).
//!
//! Sources:
//! - Architecture §32 Phase 7 Week 26 (line 4147).
//! - NIST IR 8206 §6.
//! - Mumble VoIP latency profile (~40-50ms typical mouth-to-ear).

use std::time::{Duration, Instant};

use rekindle_secrets::sframe;
use rekindle_voice::codec::{EncodedFrame, OpusCodec};
use rekindle_voice::jitter::{JitterBuffer, JitterFrame};
use rekindle_voice::mixer::AudioMixer;
use rekindle_voice::transport::VoicePacket;

#[path = "../testsupport/synth.rs"]
mod synth;
use synth::synth_frame;

use rekindle_voice::SAMPLE_RATE_HZ as SAMPLE_RATE;
const CHANNELS: u16 = 1;
use rekindle_voice::FRAME_SAMPLES_20MS as FRAME_SAMPLES;

/// Number of iterations per stage. 1000 keeps the test run fast (~1s
/// total) while giving the P95 estimate enough samples to stabilise.
const ITERATIONS: usize = 1_000;

/// Documented budget components (architecture §32 Phase 7 Week 26 +
/// `voice_config_for_group_size` defaults). Sum must stay ≤ 100ms.
struct LatencyBudget {
    /// Capture buffer — cpal fills one frame (20ms) before encode starts.
    capture: Duration,
    /// Opus VoIP algorithmic delay at 48 kHz: 6.5ms encode lookahead +
    /// 6.5ms decode lookahead. Constant property of the codec, not
    /// measured per-call.
    opus_algorithmic: Duration,
    /// Per-call wall-clock cost of encode+decode+jitter+mix. Measured
    /// here.
    pipeline_compute_p95: Duration,
    /// Jitter buffer target depth — production default per
    /// `VoiceConfig::default()`. In production this absorbs network
    /// jitter; for budget purposes it's a fixed delay added on top of
    /// compute cost.
    jitter_target: Duration,
    /// Veilid `app_message` one-way P95 over a 3-hop Tor-class
    /// `SafetySelection::Safe` route. Each safety relay adds forwarding
    /// latency (~20–30ms/hop over the open internet), so the anonymous
    /// path is ~75ms one-way vs the old ~5ms 0-hop Unsafe LAN figure.
    /// Profiled separately in `rekindle-protocol` integration tests; the
    /// value here is the documented working assumption pending live
    /// measurement.
    veilid_app_message_p95: Duration,
    /// Playback buffer fills 20ms before the speaker driver consumes
    /// the next chunk.
    playback: Duration,
}

impl LatencyBudget {
    fn total(&self) -> Duration {
        self.capture
            + self.opus_algorithmic
            + self.pipeline_compute_p95
            + self.jitter_target
            + self.veilid_app_message_p95
            + self.playback
    }
}

/// Mouth-to-ear ceiling. ITU-T G.114 puts one-way interactive voice at
/// ≤150ms "good" and ≤400ms "acceptable"; 250ms sits in the acceptable
/// band with headroom, reflecting the 3-hop anonymous voice route that
/// replaced the old 0-hop Unsafe sub-100ms path.
const MOUTH_TO_EAR_BUDGET: Duration = Duration::from_millis(250);

#[test]
fn latency_budget_holds() {
    use rekindle_voice::VoiceConfig;
    let pipeline_p95 = measure_pipeline_compute_p95();
    let production_jitter =
        Duration::from_millis(u64::from(VoiceConfig::default().jitter_buffer_ms));
    let budget = LatencyBudget {
        capture: Duration::from_millis(20),
        // 6.5ms encode + 6.5ms decode lookahead. Encoded as 13ms so we
        // don't lose precision on milliseconds boundaries; the +0.5ms
        // each side is rounded into the per-stage P95 anyway.
        opus_algorithmic: Duration::from_millis(13),
        pipeline_compute_p95: pipeline_p95,
        jitter_target: production_jitter,
        // 3-hop Tor-class Safe route one-way assumption (~25ms/hop).
        // Real production traffic crosses NAT + private routes — that
        // path is profiled separately in `rekindle-protocol` integration
        // tests; this constant exists so the budget is auditable
        // end-to-end here.
        veilid_app_message_p95: Duration::from_millis(75),
        playback: Duration::from_millis(20),
    };
    let total = budget.total();
    assert!(
        total <= MOUTH_TO_EAR_BUDGET,
        "voice mouth-to-ear budget exceeded: total={total:?} > ceiling={MOUTH_TO_EAR_BUDGET:?} \
         (ITU-T G.114 interactive ≤400ms; guard set to 250ms). Components: capture={capture:?} \
         opus_algo={opus_algo:?} compute_p95={compute:?} jitter={jitter:?} \
         veilid={veilid:?} playback={playback:?}",
        capture = budget.capture,
        opus_algo = budget.opus_algorithmic,
        compute = budget.pipeline_compute_p95,
        jitter = budget.jitter_target,
        veilid = budget.veilid_app_message_p95,
        playback = budget.playback,
    );
}

/// Measure the per-iteration wall-clock cost of one full pipeline pass
/// (encode → SFrame seal → sign + Cap'n Proto encode → decode + verify →
/// SFrame open → jitter push+pop → decode → mix), then return the P95 as
/// a `Duration`.
fn measure_pipeline_compute_p95() -> Duration {
    let mut encoder = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("encoder init");
    let mut decoder = OpusCodec::new(SAMPLE_RATE, CHANNELS, FRAME_SAMPLES).expect("decoder init");
    let mut jb = JitterBuffer::new(60);
    let mixer = AudioMixer::new(CHANNELS);
    let frame = synth_frame();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
    let sender_key = signing_key.verifying_key().to_bytes().to_vec();
    let kid = sframe::media_kid(0x1234, 0);
    let key = sframe::media_key(&[4u8; 32], &sender_key, kid);
    let mut ctr = 0u64;

    let mut one_pass = |seq: u32| -> JitterFrame {
        let encoded = encoder.encode(&frame).expect("encode");
        let timestamp = u64::from(seq) * 20;
        let metadata = VoicePacket::sframe_metadata(&sender_key, seq, timestamp, 0);
        let mut plaintext = vec![0u8];
        plaintext.extend_from_slice(&encoded.data);
        let mut packet = VoicePacket {
            sender_key: sender_key.clone(),
            sequence: seq,
            timestamp,
            transport_seq: 0,
            sframe: sframe::seal(&key, kid, ctr, &metadata, &plaintext).expect("seal"),
            sig: Vec::new(),
        };
        ctr += 1;
        packet.sign(&signing_key);
        let received = VoicePacket::decode(&packet.encode()).expect("decode packet");
        received.verify().expect("verify");
        let opened = sframe::open(&key, &received.sframe, &metadata).expect("open");
        JitterFrame {
            sequence: seq,
            timestamp,
            opus: opened[1..].to_vec(),
        }
    };

    // Pre-fill the jitter so pop returns Some on the first measured
    // iteration.
    for seq in 0..3u32 {
        let opened = one_pass(seq);
        jb.push(opened, u64::from(seq) * 20);
    }
    let mut seq: u32 = 3;

    let mut samples: Vec<Duration> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let opened = one_pass(seq);
        jb.push(opened, u64::from(seq) * 20);
        seq = seq.wrapping_add(1);
        if let Some(packet) = jb.pop(u64::from(seq) * 20) {
            let dec_frame = EncodedFrame {
                data: packet.opus,
                timestamp: packet.timestamp,
                sequence: packet.sequence,
            };
            let decoded = decoder.decode(&dec_frame).expect("decode");
            let _ = mixer.mix(&[("p0", &decoded.samples)]);
        }
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    // P95 = 95th percentile. Integer arithmetic so no float precision
    // concerns at scale.
    samples[ITERATIONS * 95 / 100]
}
