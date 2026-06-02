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

    // Best: exact channel count with a range covering the wanted rate.
    if let Some(r) = ranges
        .iter()
        .find(|r| r.channels() == want_channels && covers_rate(r))
    {
        return Some((stream_config(want_channels, want_rate), r.sample_format()));
    }
    // Next: any channel count whose range covers the wanted rate — keep
    // the rate (no resample), adapt channels only.
    if let Some(r) = ranges.iter().find(|r| covers_rate(r)) {
        return Some((stream_config(r.channels(), want_rate), r.sample_format()));
    }
    // Last resort: first advertised range at its max rate — adapt both
    // rate and channels.
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
