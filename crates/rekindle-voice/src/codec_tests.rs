use super::*;

/// Hard CBR is a security property, so it is pinned behaviourally
/// rather than by asserting we called a setter.
///
/// Under VBR, Opus frame length tracks the phoneme being encoded —
/// the published attack on encrypted VoIP recovers phrases from the
/// length sequence without touching the ciphertext. RFC 6562 says
/// such applications SHOULD run the codec in a CBR mode. This feeds
/// the encoder wildly different signals (silence, a tone, and
/// noise) and asserts the packets come out the same size, which is
/// what "carries zero information" actually means.
///
/// A regression here is silent and invisible in a call: audio still
/// works perfectly while leaking the words.
#[test]
fn cbr_makes_packet_size_independent_of_content() {
    let frame_size = 960; // 20 ms at 48 kHz
    let mut codec = OpusCodec::new(48_000, 1, frame_size).expect("codec");

    let silence = vec![0.0f32; frame_size];
    let tone: Vec<f32> = (0..frame_size)
        .map(|i| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "frame_size is 960; f32 is exact well past that"
            )]
            let t = i as f32 / 48_000.0;
            (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
        })
        .collect();
    // Deterministic pseudo-noise — the hardest case to encode, and
    // the one VBR would spend the most bits on.
    let noise: Vec<f32> = (0..frame_size)
        .map(|i| {
            let x = u32::try_from(i).unwrap_or(0).wrapping_mul(2_654_435_761);
            let hi = u16::try_from(x >> 16).unwrap_or(0);
            (f32::from(hi) / 32_768.0) - 1.0
        })
        .collect();

    // Warm up: Opus adapts over the first frames, so compare only
    // once the encoder has settled into steady state.
    for _ in 0..10 {
        codec.encode(&tone).expect("warmup");
    }

    let mut sizes = Vec::new();
    for pcm in [&silence, &tone, &noise, &silence, &noise] {
        sizes.push(codec.encode(pcm).expect("encode").data.len());
    }

    let min = sizes.iter().min().copied().unwrap_or(0);
    let max = sizes.iter().max().copied().unwrap_or(0);
    // libopus rounds each frame to a whole number of bytes, so hard
    // CBR is constant to within a byte or two, not bit-exact.
    assert!(
        max - min <= 2,
        "packet size varied with content ({min}..{max} bytes): VBR is on and the \
             encrypted stream leaks phonemes"
    );
}

/// The ladder must never drive the encoder into the region where
/// libopus documents CBR as costing noticeable quality.
///
/// `const` blocks, so lowering a constant fails the **build** rather
/// than a test run — this guards a security setting, and the whole
/// point is that it cannot be relaxed quietly.
#[test]
fn the_bitrate_floor_stays_clear_of_the_low_rate_cbr_penalty() {
    const { assert!(MIN_BITRATE_BPS >= 24_000) };
    const { assert!(DEFAULT_BITRATE_BPS >= MIN_BITRATE_BPS) };
    // Pin the worst bitrate the system can ever emit: the smallest
    // group rung (8+ peers) with a Poor-link backoff on top. The
    // ladder's own arithmetic lands under the floor there, so the
    // clamp in `report_quality_if_due` is load-bearing rather than
    // decorative — worth stating, because deleting it as redundant
    // would silently reopen the low-rate CBR quality hole.
    const {
        let worst_rung = DEFAULT_BITRATE_BPS / 2;
        assert!(
            worst_rung * 2 / 3 < MIN_BITRATE_BPS,
            "clamp is load-bearing"
        );
    };
    // So the floor is what the system actually emits at its worst.
    // (`Ord::max` is not const-callable yet, hence the runtime
    // assertion for the clamped result.)
    let worst_rung = DEFAULT_BITRATE_BPS / 2;
    assert_eq!((worst_rung * 2 / 3).max(MIN_BITRATE_BPS), MIN_BITRATE_BPS);
}

#[test]
fn test_encode_decode_roundtrip() {
    let frame_size = 960; // 20ms at 48kHz
    let mut codec = OpusCodec::new(48000, 1, frame_size).unwrap();

    // Generate a simple sine wave
    let pcm: Vec<f32> = (0i16..960)
        .map(|i| {
            let t = f32::from(i);
            (t * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5
        })
        .collect();

    let encoded = codec.encode(&pcm).unwrap();
    // Opus should compress speech-like signals significantly
    assert!(encoded.data.len() < pcm.len() * 4);

    let decoded = codec.decode(&encoded).unwrap();
    assert_eq!(decoded.samples.len(), frame_size);
}

#[test]
fn test_silence_encodes_small() {
    let frame_size = 960;
    let mut codec = OpusCodec::new(48000, 1, frame_size).unwrap();

    let silence = vec![0.0f32; frame_size];
    let encoded = codec.encode(&silence).unwrap();
    // Opus encodes silence much smaller than raw PCM (3840 bytes)
    assert!(
        encoded.data.len() < 200,
        "expected small encoded size, got {}",
        encoded.data.len()
    );
}
