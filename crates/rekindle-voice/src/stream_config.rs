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
        r.min_sample_rate() <= want_rate && r.max_sample_rate() >= want_rate
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
        sample_rate: rate,
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
    adapt(input, src_channels, None, src_rate, dst_channels, dst_rate)
}

/// [`adapt_audio`] for capture with an input-channel choice (plan C7.24b):
/// the mono downmix averages only `picked` (0-based channel indices, each
/// below `src_channels`), so a mic on input 1 of a two-input interface is
/// not halved by a silent instrument input. Mumble's channel mask
/// (`AudioInput.cpp` `inMixerFloatMask`) averages its selected channels
/// the same way.
pub fn adapt_capture(
    input: &[f32],
    src_channels: u16,
    picked: &[u16],
    src_rate: u32,
    dst_channels: u16,
    dst_rate: u32,
) -> Vec<f32> {
    adapt(
        input,
        src_channels,
        Some(picked),
        src_rate,
        dst_channels,
        dst_rate,
    )
}

fn adapt(
    input: &[f32],
    src_channels: u16,
    picked: Option<&[u16]>,
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

    // 1. Downmix to mono: the picked channels, or all of them.
    let mono: Vec<f32> = match picked {
        Some(picked) if !picked.is_empty() => {
            let n = f32::from(u16::try_from(picked.len()).unwrap_or(u16::MAX));
            input
                .chunks(src_ch_n)
                .map(|frame| {
                    picked
                        .iter()
                        .filter_map(|&c| frame.get(usize::from(c)))
                        .sum::<f32>()
                        / n
                })
                .collect()
        }
        _ if src_ch_n == 1 => input.to_vec(),
        _ => input
            .chunks(src_ch_n)
            .map(|frame| frame.iter().sum::<f32>() / f32::from(src_ch))
            .collect(),
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
#[path = "stream_config_tests.rs"]
mod tests;
