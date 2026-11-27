use super::*;

fn range(
    channels: u16,
    min_rate: u32,
    max_rate: u32,
    format: cpal::SampleFormat,
) -> cpal::SupportedStreamConfigRange {
    cpal::SupportedStreamConfigRange::new(
        channels,
        min_rate,
        max_rate,
        cpal::SupportedBufferSize::Unknown,
        format,
    )
}

#[test]
fn unopenable_format_loses_to_openable_one_at_same_rate() {
    // The live failure: pipewire-alsa advertises a U8 range first
    // while also offering f32 — U8 must never win.
    let ranges = vec![
        range(1, 8000, 48000, cpal::SampleFormat::U8),
        range(1, 8000, 48000, cpal::SampleFormat::F32),
    ];
    let (config, format) = pick_supported(ranges.into_iter(), 48000, 1).unwrap();
    assert_eq!(format, cpal::SampleFormat::F32);
    assert_eq!(config.sample_rate, 48000);
    assert_eq!(config.channels, 1);
}

#[test]
fn f32_preferred_over_i16_over_u16() {
    let ranges = vec![
        range(1, 8000, 48000, cpal::SampleFormat::U16),
        range(1, 8000, 48000, cpal::SampleFormat::I16),
        range(1, 8000, 48000, cpal::SampleFormat::F32),
    ];
    let (_, format) = pick_supported(ranges.into_iter(), 48000, 1).unwrap();
    assert_eq!(format, cpal::SampleFormat::F32);
}

#[test]
fn openable_format_beats_exact_channel_match() {
    // A stereo f32 range must win over a mono U8 range even though
    // mono matches the wanted channel count — an unopenable format
    // is useless no matter how well the rate/channels line up.
    let ranges = vec![
        range(1, 8000, 48000, cpal::SampleFormat::U8),
        range(2, 8000, 48000, cpal::SampleFormat::F32),
    ];
    let (config, format) = pick_supported(ranges.into_iter(), 48000, 1).unwrap();
    assert_eq!(format, cpal::SampleFormat::F32);
    assert_eq!(config.channels, 2);
    assert_eq!(config.sample_rate, 48000);
}

#[test]
fn openable_off_rate_range_beats_unopenable_covering_range() {
    // Only a 44.1 kHz i16 range is openable; the 48 kHz U8 range
    // covering the wanted rate must not win.
    let ranges = vec![
        range(1, 8000, 48000, cpal::SampleFormat::U8),
        range(2, 44100, 44100, cpal::SampleFormat::I16),
    ];
    let (config, format) = pick_supported(ranges.into_iter(), 48000, 1).unwrap();
    assert_eq!(format, cpal::SampleFormat::I16);
    assert_eq!(config.sample_rate, 44100);
}

#[test]
fn all_unopenable_falls_back_to_first_range() {
    // Nothing openable: keep the old behavior so the stream builder
    // reports the precise "unsupported sample format" diagnostic.
    let ranges = vec![range(1, 8000, 48000, cpal::SampleFormat::U8)];
    let (_, format) = pick_supported(ranges.into_iter(), 48000, 1).unwrap();
    assert_eq!(format, cpal::SampleFormat::U8);
}

#[test]
fn identity_when_formats_match() {
    let input = vec![0.1, -0.2, 0.3, -0.4];
    assert_eq!(adapt_audio(&input, 1, 48000, 1, 48000), input);
}

#[test]
fn empty_input_yields_empty() {
    assert!(adapt_audio(&[], 2, 44100, 1, 48000).is_empty());
}

#[test]
fn downmix_stereo_to_mono_averages_channels() {
    // Interleaved L/R frames: (1.0,0.0),(0.0,1.0) → averages 0.5,0.5.
    let out = adapt_audio(&[1.0, 0.0, 0.0, 1.0], 2, 48000, 1, 48000);
    assert_eq!(out, vec![0.5, 0.5]);
}

/// Plan C7.24b — picking input 1 of a two-input interface keeps the mic
/// at full level instead of averaging it with a silent second input.
#[test]
fn picked_channels_downmix_only_those() {
    // Two frames: mic on channel 0, silence on channel 1.
    let input = [0.8, 0.0, -0.4, 0.0];
    assert_eq!(adapt_audio(&input, 2, 48000, 1, 48000), vec![0.4, -0.2]);
    assert_eq!(
        adapt_capture(&input, 2, &[0], 48000, 1, 48000),
        vec![0.8, -0.4]
    );
    assert_eq!(
        adapt_capture(&input, 2, &[1], 48000, 1, 48000),
        vec![0.0, 0.0]
    );
    assert_eq!(
        adapt_capture(&input, 2, &[0, 1], 48000, 1, 48000),
        adapt_audio(&input, 2, 48000, 1, 48000)
    );
}

#[test]
fn upmix_mono_to_stereo_duplicates_samples() {
    let out = adapt_audio(&[0.5, -0.5], 1, 48000, 2, 48000);
    assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
}

#[test]
fn upsample_doubles_length() {
    // 24 kHz → 48 kHz roughly doubles the sample count.
    let input = vec![0.0, 1.0, 0.0, -1.0];
    let out = adapt_audio(&input, 1, 24000, 1, 48000);
    assert_eq!(out.len(), 8);
    // First sample preserved; interpolation stays within range.
    assert!((out[0] - 0.0).abs() < 1e-6);
    assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
}

#[test]
fn downsample_halves_length() {
    let input = vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25];
    let out = adapt_audio(&input, 1, 48000, 1, 24000);
    assert_eq!(out.len(), 4);
}
