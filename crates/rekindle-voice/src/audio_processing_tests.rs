use super::*;

#[test]
fn test_silence_not_speech() {
    let mut proc = AudioProcessor::new(false, false, 0.5, 100, 20);
    let silence = vec![0.0f32; 960];
    let result = proc.process_capture(&silence, None);
    assert!(!result.is_speech);
    assert_eq!(result.samples.len(), 960);
}

#[test]
fn test_noise_suppression_scaling() {
    // Verify output is normalized [-1, 1] despite internal 16-bit scaling
    let mut proc = AudioProcessor::new(true, false, 0.01, 100, 20);
    let signal: Vec<f32> = (0i16..960)
        .map(|i| {
            let t = f32::from(i);
            (t * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.3
        })
        .collect();
    let result = proc.process_capture(&signal, None);
    assert_eq!(result.samples.len(), 960);

    // All output samples must be in normalized range
    for &sample in &result.samples {
        assert!(
            (-1.0..=1.0).contains(&sample),
            "sample {sample} out of normalized range"
        );
    }
}

#[test]
fn test_with_echo_cancellation() {
    let mut proc = AudioProcessor::new(true, true, 0.5, 100, 20);
    let silence = vec![0.0f32; 960];
    let speaker = vec![0.0f32; 960];

    // Feed speaker reference
    proc.feed_speaker_reference(&speaker);

    // Process with speaker reference — should not panic
    let result = proc.process_capture(&silence, Some(&speaker));
    assert_eq!(result.samples.len(), 960);
}

#[test]
fn no_speaker_reference_does_not_crush_near_end() {
    // Regression: AEC3 with NO render reference has no echo to model and
    // adapts toward suppressing the near-end, crushing the mic toward
    // silence over a second or so (observed live: raw_peak ~0.24 ->
    // proc_peak ~0.0008). The guard bypasses AEC when there's no
    // reference. noise_suppression=false so the output reflects only the
    // echo-cancellation stage; echo_cancellation=true builds the AEC.
    let mut proc = AudioProcessor::new(false, true, 0.01, 100, 20);
    let frame: Vec<f32> = (0i16..960)
        .map(|i| (f32::from(i) * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5)
        .collect();
    let raw_peak = frame.iter().fold(0.0f32, |m, &s| m.max(s.abs()));

    // Run ~1s of frames with NO render reference; the old (unguarded) AEC
    // path would have adapted to near-silence by the end.
    let mut last_peak = 0.0f32;
    for _ in 0..50 {
        let result = proc.process_capture(&frame, None);
        last_peak = result.samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    }
    assert!(
        last_peak > raw_peak * 0.5,
        "near-end crushed without a render reference: raw_peak={raw_peak}, last_peak={last_peak}"
    );
}
