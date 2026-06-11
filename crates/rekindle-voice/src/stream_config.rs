//! cpal stream-config negotiation + format adaptation.
//!
//! The pipeline (Opus encoder/decoder + mixer) runs at a fixed
//! `VoiceConfig` rate/channel count (48 kHz mono by default). Real
//! capture/playback devices frequently refuse that exact `StreamConfig`
//! with "the requested stream configuration is not supported by the
//! device" — they advertise a different native rate, channel count, or
//! sample format. We negotiate a config the device actually supports
//! and adapt buffers between the device format and the pipeline format.
//! This is the capture-native-then-resample approach WebRTC and Mumble
//! use; cpal upstream recommends the same (query `supported_*_configs`,
//! fall back to `default_*_config`, resample to the codec rate).

use cpal::traits::DeviceTrait;

use crate::error::VoiceError;

/// Pick a cpal input config the device supports, preferring the
/// pipeline's exact `(channels, sample_rate)`. Falls back through any
/// supported config covering the wanted rate, then the device default
/// (always supported). Callers adapt buffers via [`adapt_audio`] when
/// the chosen config differs from the pipeline format.
pub fn negotiate_input_config(
    device: &cpal::Device,
    want_rate: u32,
    want_channels: u16,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), VoiceError> {
    if let Ok(ranges) = device.supported_input_configs() {
        if let Some(chosen) = pick_supported(ranges, want_rate, want_channels) {
            return Ok(chosen);
        }
    }
    let def = device
        .default_input_config()
        .map_err(|e| VoiceError::AudioDevice(format!("no input config: {e}")))?;
    Ok((def.config(), def.sample_format()))
}

/// Output counterpart of [`negotiate_input_config`].
pub fn negotiate_output_config(
    device: &cpal::Device,
    want_rate: u32,
    want_channels: u16,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), VoiceError> {
    if let Ok(ranges) = device.supported_output_configs() {
        if let Some(chosen) = pick_supported(ranges, want_rate, want_channels) {
            return Ok(chosen);
        }
    }
    let def = device
        .default_output_config()
        .map_err(|e| VoiceError::AudioDevice(format!("no output config: {e}")))?;
    Ok((def.config(), def.sample_format()))
}

/// Rank of a sample format the capture/playback builders can actually
/// open (their `match sample_format` arms): lower is better, `None` is
/// unopenable. F32 is the pipeline's native format (no conversion),
/// I16 a lossless-enough integer path, U16 the awkward offset-binary
/// case. Everything else (U8, I8, F64, …) errors at stream build, so a
/// rate-perfect range in such a format is worth less than ANY openable
/// range — devices bridged through pipewire-alsa advertise U8 ranges
/// first while also offering f32/i16 (the "unsupported sample format:
/// U8" dead-voice failure).
fn format_rank(format: cpal::SampleFormat) -> Option<u8> {
    match format {
        cpal::SampleFormat::F32 => Some(0),
        cpal::SampleFormat::I16 => Some(1),
        cpal::SampleFormat::U16 => Some(2),
        _ => None,
    }
}

fn pick_supported<I>(
    ranges: I,
    want_rate: u32,
    want_channels: u16,
) -> Option<(cpal::StreamConfig, cpal::SampleFormat)>
where
    I: Iterator<Item = cpal::SupportedStreamConfigRange>,
{
    let ranges: Vec<_> = ranges.collect();
    let covers_rate = |r: &cpal::SupportedStreamConfigRange| {
        r.min_sample_rate().0 <= want_rate && r.max_sample_rate().0 >= want_rate
    };
    // Openable format first, then the existing rate/channel preference
    // within each tier; `min_by_key` keeps the best-ranked format.
    let best_openable = |pred: &dyn Fn(&cpal::SupportedStreamConfigRange) -> bool| {
        ranges
            .iter()
            .filter(|r| pred(r))
            .filter_map(|r| format_rank(r.sample_format()).map(|rank| (rank, r)))
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, r)| r)
    };

    // Best: exact channel count with a range covering the wanted rate.
    if let Some(r) = best_openable(&|r| r.channels() == want_channels && covers_rate(r)) {
        return Some((stream_config(want_channels, want_rate), r.sample_format()));
    }
    // Next: any channel count whose range covers the wanted rate — keep
    // the rate (no resample), adapt channels only.
    if let Some(r) = best_openable(&|r| covers_rate(r)) {
        return Some((stream_config(r.channels(), want_rate), r.sample_format()));
    }
    // Next: any openable range at its max rate — adapt both rate and
    // channels.
    if let Some(r) = best_openable(&|_| true) {
        let chosen = (*r).with_max_sample_rate();
        return Some((chosen.config(), chosen.sample_format()));
    }
    // Last resort: first advertised range at its max rate — unopenable
    // format, but preserves the original diagnostic ("unsupported
    // sample format: …") over a less precise "no input config".
    ranges.first().map(|r| {
        let chosen = (*r).with_max_sample_rate();
        (chosen.config(), chosen.sample_format())
    })
}

fn stream_config(channels: u16, rate: u32) -> cpal::StreamConfig {
    cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(rate),
        buffer_size: cpal::BufferSize::Default,
    }
}

/// Convert an interleaved f32 buffer from `(src_channels, src_rate)` to
/// `(dst_channels, dst_rate)`: downmix to mono, linear-resample, then
/// fan out to the destination channel count. Identity when the formats
/// already match. Linear interpolation is adequate for the rare
/// device-mismatch fallback — the happy path negotiates an exact match
/// and never calls this.
pub fn adapt_audio(
    input: &[f32],
    src_channels: u16,
    src_rate: u32,
    dst_channels: u16,
    dst_rate: u32,
) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let src_ch = src_channels.max(1);
    let dst_ch = dst_channels.max(1);
    let src_ch_n = usize::from(src_ch);
    let dst_ch_n = usize::from(dst_ch);

    // 1. Downmix to mono.
    let mono: Vec<f32> = if src_ch_n == 1 {
        input.to_vec()
    } else {
        input
            .chunks(src_ch_n)
            .map(|frame| frame.iter().sum::<f32>() / f32::from(src_ch))
            .collect()
    };

    // 2. Linear-resample mono to dst_rate. Integer fixed-point position
    //    math keeps every conversion lossless (no float→int casts): the
    //    sample index is exact and the interpolation weight is a Q16
    //    fraction (1/65536 resolution — far below audible for the rare
    //    device-mismatch fallback).
    let resampled: Vec<f32> = if src_rate == dst_rate {
        mono
    } else {
        let last = mono.len() - 1;
        let src_rate = u64::from(src_rate);
        let dst_rate = u64::from(dst_rate);
        let out_len = usize::try_from(mono.len() as u64 * dst_rate / src_rate)
            .unwrap_or(mono.len())
            .max(1);
        (0..out_len)
            .map(|i| {
                // Source position (fixed-point): pos = i * src_rate / dst_rate.
                let pos_num = i as u64 * src_rate;
                let idx = usize::try_from(pos_num / dst_rate).unwrap_or(last);
                let rem = pos_num % dst_rate;
                let frac =
                    f32::from(u16::try_from(rem * 65_536 / dst_rate).unwrap_or(0)) / 65_536.0;
                let a = mono[idx.min(last)];
                let b = mono[(idx + 1).min(last)];
                a + (b - a) * frac
            })
            .collect()
    };

    // 3. Fan mono out to dst_channels (interleaved).
    if dst_ch_n == 1 {
        resampled
    } else {
        let mut out = Vec::with_capacity(resampled.len() * dst_ch_n);
        for s in resampled {
            for _ in 0..dst_ch_n {
                out.push(s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(
        channels: u16,
        min_rate: u32,
        max_rate: u32,
        format: cpal::SampleFormat,
    ) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            cpal::SampleRate(min_rate),
            cpal::SampleRate(max_rate),
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
        assert_eq!(config.sample_rate.0, 48000);
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
        assert_eq!(config.sample_rate.0, 48000);
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
        assert_eq!(config.sample_rate.0, 44100);
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
}
