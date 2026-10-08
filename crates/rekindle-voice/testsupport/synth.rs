//! Test-signal synthesis shared by the latency test and the latency
//! bench.
//!
//! `tests/` and `benches/` compile as separate crates, so neither can
//! import from the other; each carried a byte-identical `synth_frame`.
//! That matters more than the eleven duplicated lines: the bench exists
//! to measure the pipeline the budget test validates, so if the two
//! drift apart they stop describing the same workload. Both `#[path]`-
//! include this file instead.

/// One 20 ms frame of a 440 Hz sine at half amplitude.
pub fn synth_frame() -> Vec<f32> {
    let two_pi_freq = 2.0 * std::f32::consts::PI * 440.0;
    let sample_rate = f32::from(u16::try_from(rekindle_voice::SAMPLE_RATE_HZ).unwrap_or(u16::MAX));
    let inv_sample_rate = 1.0_f32 / sample_rate;
    (0..rekindle_voice::FRAME_SAMPLES_20MS)
        .map(|i| {
            let i = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
            (two_pi_freq * i * inv_sample_rate).sin() * 0.5
        })
        .collect()
}
