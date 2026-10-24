//! Voice messages (plan E5.1; prod research batch 7 V-1/V-2): the Host owns
//! the format. A recording arrives as mono PCM, whatever the webview's
//! recorder produced, and leaves as Opus in Ogg (RFC 7845) with an empty
//! `OpusTags` (no vendor, no comments), so the label (`audio/ogg`), the
//! container and the playback agree on every OS. A received voice message
//! is decoded once here, and its duration and waveform come from the audio
//! itself, never from what the sender said. It uses the `opus` binding
//! the ADR 0015 media engine (`rekindle-voice`) encodes calls with.

mod ogg;
#[cfg(test)]
mod tests;

use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use thiserror::Error;

/// The sample rate of the PCM a recording is handed over at.
pub const INPUT_SAMPLE_RATE: u32 = 16_000;
/// The longest voice message, in ms.
pub const MAX_DURATION_MS: u32 = 5 * 60 * 1000;
/// The shortest voice message, in ms (a shorter press is a cancel).
pub const MIN_DURATION_MS: u32 = 200;
/// How many peaks a waveform has.
pub const WAVEFORM_PEAKS: usize = 64;
/// The MIME type every voice message carries.
pub const MIME_TYPE: &str = "audio/ogg";

/// Opus granule positions count 48 kHz samples (RFC 7845 §4).
const GRANULE_RATE: u64 = 48_000;
/// 20 ms frames at [`INPUT_SAMPLE_RATE`].
const FRAME_SAMPLES: usize = 320;
/// ≈ Signal's and SimpleX's 32 kbps AAC (research V-1: 16–24 kbps Opus).
const BITRATE: i32 = 24_000;
/// Largest Opus packet (RFC 6716 §3.4).
const MAX_PACKET: usize = 1275;
/// Audio packets per page: one second of 20 ms frames.
const PACKETS_PER_PAGE: usize = 50;
/// The empty `OpusTags` header: magic, vendor length 0, comment count 0.
const EMPTY_TAGS: &[u8; 16] = b"OpusTags\0\0\0\0\0\0\0\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VoiceNoteError {
    #[error("voice message is {got_ms} ms; it must be {MIN_DURATION_MS}..={MAX_DURATION_MS} ms")]
    Duration { got_ms: u64 },
    #[error("PCM is not whole 32-bit float samples")]
    Pcm,
    #[error("not an Ogg Opus voice message: {0}")]
    Format(String),
    #[error("opus: {0}")]
    Codec(String),
}

/// A received voice message, as decoded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceNoteInfo {
    pub duration_ms: u32,
    /// [`WAVEFORM_PEAKS`] peaks, 0–255.
    pub waveform: Vec<u8>,
}

fn codec(e: &opus::Error) -> VoiceNoteError {
    VoiceNoteError::Codec(e.to_string())
}

fn duration_ms(samples: u64, rate: u64) -> u64 {
    samples * 1000 / rate
}

fn check_duration(ms: u64) -> Result<u32, VoiceNoteError> {
    u32::try_from(ms)
        .ok()
        .filter(|ms| (MIN_DURATION_MS..=MAX_DURATION_MS).contains(ms))
        .ok_or(VoiceNoteError::Duration { got_ms: ms })
}

/// Read handed-over PCM: little-endian `f32` mono samples at
/// [`INPUT_SAMPLE_RATE`].
///
/// # Errors
/// The length is not a whole number of samples.
pub fn pcm_from_le_bytes(bytes: &[u8]) -> Result<Vec<f32>, VoiceNoteError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(VoiceNoteError::Pcm);
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Encode a recording as Opus in Ogg: mono, [`BITRATE`], 20 ms frames, an
/// empty `OpusTags`. The stream serial is derived from the audio, so the
/// file carries nothing about who or what made it.
///
/// # Errors
/// The recording is shorter than [`MIN_DURATION_MS`] or longer than
/// [`MAX_DURATION_MS`], or the encoder failed.
pub fn encode(pcm: &[f32]) -> Result<Vec<u8>, VoiceNoteError> {
    let samples = u64::try_from(pcm.len()).map_err(|_| VoiceNoteError::Pcm)?;
    check_duration(duration_ms(samples, u64::from(INPUT_SAMPLE_RATE)))?;
    let mut encoder = Encoder::new(INPUT_SAMPLE_RATE, Channels::Mono, Application::Voip)
        .map_err(|e| codec(&e))?;
    encoder
        .set_bitrate(Bitrate::Bits(BITRATE))
        .map_err(|e| codec(&e))?;
    let scale = GRANULE_RATE / u64::from(INPUT_SAMPLE_RATE);
    let lookahead = u64::try_from(encoder.get_lookahead().map_err(|e| codec(&e))?).unwrap_or(0);
    let pre_skip = u16::try_from(lookahead * scale).map_err(|_| VoiceNoteError::Pcm)?;

    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1);
    head.push(1);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&INPUT_SAMPLE_RATE.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);

    let digest = blake3::hash(bytemuck_f32(pcm).as_slice());
    let serial = u32::from_le_bytes([
        digest.as_bytes()[0],
        digest.as_bytes()[1],
        digest.as_bytes()[2],
        digest.as_bytes()[3],
    ]);
    let mut writer = ogg::OggWriter::new(serial);
    writer.page(&[&head], 0, true, false);
    writer.page(&[EMPTY_TAGS], 0, false, false);

    // Encode every frame (the last zero-padded), plus enough silence to
    // flush the lookahead; the last page's granule trims the padding.
    let total = pcm.len() + usize::try_from(lookahead).unwrap_or(0);
    let frames = total.div_ceil(FRAME_SAMPLES);
    let end_granule = u64::from(pre_skip) + samples * scale;
    let mut buf = [0u8; MAX_PACKET];
    let mut page: Vec<Vec<u8>> = Vec::new();
    let mut segments = 0usize;
    let mut granule = 0u64;
    for index in 0..frames {
        let start = index * FRAME_SAMPLES;
        let mut frame = [0f32; FRAME_SAMPLES];
        if start < pcm.len() {
            let end = (start + FRAME_SAMPLES).min(pcm.len());
            frame[..end - start].copy_from_slice(&pcm[start..end]);
        }
        let len = encoder
            .encode_float(&frame, &mut buf)
            .map_err(|e| codec(&e))?;
        let packet = buf[..len].to_vec();
        if segments + ogg::OggWriter::segments(&packet) > ogg::OggWriter::MAX_LACING_VALUES
            || page.len() == PACKETS_PER_PAGE
        {
            let refs: Vec<&[u8]> = page.iter().map(Vec::as_slice).collect();
            writer.page(&refs, granule, false, false);
            page.clear();
            segments = 0;
        }
        segments += ogg::OggWriter::segments(&packet);
        page.push(packet);
        let encoded = u64::try_from((index + 1) * FRAME_SAMPLES).unwrap_or(u64::MAX);
        granule = (encoded * scale).min(end_granule);
    }
    let refs: Vec<&[u8]> = page.iter().map(Vec::as_slice).collect();
    writer.page(&refs, end_granule, false, true);
    Ok(writer.finish())
}

fn bytemuck_f32(pcm: &[f32]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// Decode a received voice message: it must be one Ogg stream of mono
/// Opus with an empty `OpusTags`, between [`MIN_DURATION_MS`] and
/// [`MAX_DURATION_MS`] long. Its duration comes from the last granule
/// position and its waveform from the decoded samples.
///
/// # Errors
/// Anything else.
pub fn analyze(bytes: &[u8]) -> Result<VoiceNoteInfo, VoiceNoteError> {
    let format = |why: &str| VoiceNoteError::Format(why.to_string());
    let packets = ogg::read(bytes).map_err(VoiceNoteError::Format)?;
    let mut packets = packets.into_iter();
    let head = packets.next().ok_or_else(|| format("no OpusHead"))?;
    let head = &head.data;
    if head.len() != 19
        || &head[..8] != b"OpusHead"
        || head[8] != 1
        || head[9] != 1
        || head[18] != 0
    {
        return Err(format("not a mono OpusHead (version 1, mapping 0)"));
    }
    let pre_skip = u64::from(u16::from_le_bytes([head[10], head[11]]));
    let tags = packets.next().ok_or_else(|| format("no OpusTags"))?;
    if tags.data.as_slice() != EMPTY_TAGS {
        return Err(format("OpusTags is not empty"));
    }
    let mut decoder = Decoder::new(48_000, Channels::Mono).map_err(|e| codec(&e))?;
    let mut decoded: Vec<i16> = Vec::new();
    let mut last_granule = None;
    let mut out = vec![0i16; 5760];
    for packet in packets {
        let n = decoder
            .decode(&packet.data, &mut out, false)
            .map_err(|e| codec(&e))?;
        decoded.extend_from_slice(&out[..n]);
        if packet.page_granule.is_some() {
            last_granule = packet.page_granule;
        }
    }
    let granule = last_granule.ok_or_else(|| format("no audio"))?;
    let samples = granule
        .checked_sub(pre_skip)
        .ok_or_else(|| format("granule before pre-skip"))?;
    let duration = check_duration(duration_ms(samples, GRANULE_RATE))?;
    let audio = decoded
        .get(usize::try_from(pre_skip).unwrap_or(usize::MAX)..)
        .and_then(|rest| rest.get(..usize::try_from(samples).unwrap_or(usize::MAX)))
        .ok_or_else(|| format("granule past the decoded audio"))?;
    Ok(VoiceNoteInfo {
        duration_ms: duration,
        waveform: waveform(audio),
    })
}

/// [`WAVEFORM_PEAKS`] peaks of `audio`, each its bucket's largest
/// magnitude scaled to 0–255.
fn waveform(audio: &[i16]) -> Vec<u8> {
    let bucket = audio.len().div_ceil(WAVEFORM_PEAKS).max(1);
    (0..WAVEFORM_PEAKS)
        .map(|i| {
            let peak = audio
                .iter()
                .skip(i * bucket)
                .take(bucket)
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap_or(0);
            u8::try_from(peak >> 7).unwrap_or(u8::MAX)
        })
        .collect()
}
