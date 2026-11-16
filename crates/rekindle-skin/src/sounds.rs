//! Validator step 5: sounds (plan E7.7.1).
//!
//! Every file under `sounds/` is a sound cue: WAV (PCM), Ogg Vorbis or
//! FLAC, declared by its extension. Only the declared container's reader
//! and codec's decoder run (`symphonia`, pure Rust; the format is never
//! probed from the bytes), and the stream must be one audio track of at
//! most [`MAX_SAMPLE_RATE`] Hz and [`MAX_SOUND_CHANNELS`] channels. The
//! decode stops as soon as it passes [`MAX_SOUND_SECONDS`]. The samples are
//! re-encoded with `hound` to a PCM16 WAV, so no tag, cue list or other
//! chunk of the source survives. At most [`MAX_SOUND_CUES`] cues.
//!
//! The manifest's sounds must name files under `sounds/` that the package
//! holds.

use std::collections::BTreeMap;
use std::io::{Cursor, ErrorKind};

use rekindle_types::dto::skin::{SkinError, SkinManifest, SkinRule};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as AudioError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::codecs::{FlacDecoder, PcmDecoder, VorbisDecoder};
use symphonia::default::formats::{FlacReader, OggReader, WavReader};

use crate::images::extension;
use crate::limits::{MAX_SAMPLE_RATE, MAX_SOUND_CHANNELS, MAX_SOUND_CUES, MAX_SOUND_SECONDS};
use crate::package::{Package, MANIFEST};

/// The directory whose files are sound cues.
pub const SOUND_DIR: &str = "sounds/";

/// The containers (and their one codec) a cue may be.
#[derive(Clone, Copy)]
enum SoundKind {
    Wav,
    OggVorbis,
    Flac,
}

/// A decoded cue: interleaved 16-bit samples.
struct Pcm {
    rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

fn declared_kind(name: &str) -> Result<SoundKind, SkinRule> {
    match extension(name).as_deref() {
        Some("wav") => Ok(SoundKind::Wav),
        Some("ogg") => Ok(SoundKind::OggVorbis),
        Some("flac") => Ok(SoundKind::Flac),
        _ => Err(SkinRule::UnsupportedEntryType {
            expected: "a .wav, .ogg (Vorbis) or .flac sound".to_owned(),
        }),
    }
}

/// Checks every sound cue and returns each one's canonical PCM16 WAV, by
/// entry name.
///
/// # Errors
/// The first rule a cue or a manifest reference breaks.
pub fn validate(
    package: &Package<'_>,
    manifest: &SkinManifest,
) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    check_references(package, manifest)?;
    let cues: Vec<_> = package
        .files()
        .filter(|(name, _)| name.starts_with(SOUND_DIR))
        .collect();
    let count = cues.len() as u64;
    if count > MAX_SOUND_CUES {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::TooManySoundCues {
                count,
                max: MAX_SOUND_CUES,
            },
        ));
    }
    let mut out = BTreeMap::new();
    for (name, bytes) in cues {
        let wav = check(name, bytes).map_err(|rule| SkinError::new(name, rule))?;
        out.insert(name.to_owned(), wav);
    }
    Ok(out)
}

/// One cue alone, as an importer checks a legacy sound before it enters a
/// package (plan E7.7.3): the declared container's reader, the track and
/// length limits, and the PCM16 WAV re-encode of [`validate`].
///
/// # Errors
/// The first rule the cue breaks.
pub fn check(name: &str, bytes: &[u8]) -> Result<Vec<u8>, SkinRule> {
    declared_kind(name)
        .and_then(|kind| decode(bytes, kind))
        .and_then(|pcm| encode(&pcm))
}

fn decode_error(detail: &dyn std::fmt::Display) -> SkinRule {
    SkinRule::SoundDecode {
        detail: detail.to_string(),
    }
}

fn reader(bytes: &[u8], kind: SoundKind) -> Result<Box<dyn FormatReader>, AudioError> {
    let source = MediaSourceStream::new(
        Box::new(Cursor::new(bytes.to_vec())),
        MediaSourceStreamOptions::default(),
    );
    let options = FormatOptions::default();
    Ok(match kind {
        SoundKind::Wav => Box::new(WavReader::try_new(source, &options)?),
        SoundKind::OggVorbis => Box::new(OggReader::try_new(source, &options)?),
        SoundKind::Flac => Box::new(FlacReader::try_new(source, &options)?),
    })
}

fn decode(bytes: &[u8], kind: SoundKind) -> Result<Pcm, SkinRule> {
    let mut reader = reader(bytes, kind).map_err(|e| decode_error(&e))?;
    let [track] = reader.tracks() else {
        return Err(decode_error(&"not exactly one track"));
    };
    let params = track.codec_params.clone();
    let rate = params
        .sample_rate
        .ok_or_else(|| decode_error(&"no sample rate"))?;
    if rate > MAX_SAMPLE_RATE {
        return Err(SkinRule::SampleRateTooHigh {
            rate,
            max: MAX_SAMPLE_RATE,
        });
    }
    let channels = params
        .channels
        .map(|c| u32::try_from(c.count()).unwrap_or(u32::MAX))
        .ok_or_else(|| decode_error(&"no channel layout"))?;
    if channels > MAX_SOUND_CHANNELS {
        return Err(SkinRule::TooManyChannels {
            channels,
            max: MAX_SOUND_CHANNELS,
        });
    }
    let too_long = SkinRule::SoundTooLong {
        max_seconds: MAX_SOUND_SECONDS,
    };
    let max_frames = u64::from(rate) * u64::from(MAX_SOUND_SECONDS);
    if params.n_frames.is_some_and(|n| n > max_frames) {
        return Err(too_long);
    }
    let options = DecoderOptions { verify: true };
    let mut decoder: Box<dyn Decoder> = match kind {
        SoundKind::Wav => {
            Box::new(PcmDecoder::try_new(&params, &options).map_err(|e| decode_error(&e))?)
        }
        SoundKind::OggVorbis => {
            Box::new(VorbisDecoder::try_new(&params, &options).map_err(|e| decode_error(&e))?)
        }
        SoundKind::Flac => {
            Box::new(FlacDecoder::try_new(&params, &options).map_err(|e| decode_error(&e))?)
        }
    };
    let mut samples = Vec::new();
    let mut frames = 0u64;
    loop {
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(AudioError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(decode_error(&e)),
        };
        let buffer = decoder.decode(&packet).map_err(|e| decode_error(&e))?;
        let spec = *buffer.spec();
        if spec.rate != rate || u32::try_from(spec.channels.count()).ok() != Some(channels) {
            return Err(decode_error(&"the format changes mid-stream"));
        }
        frames += buffer.frames() as u64;
        if frames > max_frames {
            return Err(too_long);
        }
        let mut interleaved = SampleBuffer::<i16>::new(buffer.capacity() as u64, spec);
        interleaved.copy_interleaved_ref(buffer);
        samples.extend_from_slice(interleaved.samples());
    }
    if frames == 0 || channels == 0 {
        return Err(decode_error(&"no audio"));
    }
    Ok(Pcm {
        rate,
        channels: u16::try_from(channels).map_err(|e| decode_error(&e))?,
        samples,
    })
}

/// The samples as a PCM16 WAV: `fmt ` and `data` chunks only.
fn encode(pcm: &Pcm) -> Result<Vec<u8>, SkinRule> {
    let encode_error = |e: hound::Error| SkinRule::SoundEncode {
        detail: e.to_string(),
    };
    let spec = hound::WavSpec {
        channels: pcm.channels,
        sample_rate: pcm.rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut out, spec).map_err(encode_error)?;
    for sample in &pcm.samples {
        writer.write_sample(*sample).map_err(encode_error)?;
    }
    writer.finalize().map_err(encode_error)?;
    Ok(out.into_inner())
}

fn check_references(package: &Package<'_>, manifest: &SkinManifest) -> Result<(), SkinError> {
    let count = manifest.sounds.len() as u64;
    if count > MAX_SOUND_CUES {
        return Err(SkinError::new(
            MANIFEST,
            SkinRule::TooManySoundCues {
                count,
                max: MAX_SOUND_CUES,
            },
        ));
    }
    for (id, path) in &manifest.sounds {
        let field = format!("sounds.{id}");
        if !path.starts_with(SOUND_DIR) {
            return Err(SkinError::new(
                MANIFEST,
                SkinRule::BadValue {
                    field,
                    expected: format!("an entry under {SOUND_DIR}"),
                },
            ));
        }
        if package.get(path).is_none() {
            return Err(SkinError::new(
                path.as_str(),
                SkinRule::MissingEntry { field },
            ));
        }
    }
    Ok(())
}
