//! Validator step 5: one crafted cue per refusal, and the PCM16 WAV
//! re-encode (plan E7.7.1 Verify).

use std::collections::BTreeMap;
use std::io::Cursor;

use rekindle_skin::limits::{
    MAX_SAMPLE_RATE, MAX_SOUND_CHANNELS, MAX_SOUND_CUES, MAX_SOUND_SECONDS,
};
use rekindle_skin::{manifest, package, sounds};
use rekindle_types::dto::skin::{SkinError, SkinRule};
use serde_json::json;

use crate::common::{self, Entry};

fn run_with(
    manifest_json: &serde_json::Value,
    extra: &[Entry],
) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    let mut entries = vec![
        Entry::file("manifest.json", manifest_json.to_string().as_bytes()),
        Entry::file("dark.json", b"{}"),
    ];
    entries.extend_from_slice(extra);
    let bytes = common::zip(&entries);
    let package = package::read(&bytes)?;
    let manifest = manifest::parse(&package)?;
    sounds::validate(&package, &manifest)
}

fn run(extra: &[Entry]) -> Result<BTreeMap<String, Vec<u8>>, SkinError> {
    run_with(&common::manifest(), extra)
}

fn refused(extra: &[Entry]) -> SkinRule {
    run(extra).expect_err("the cue must be refused").rule
}

/// A WAV of `frames` frames, 16-bit integer samples.
fn wav(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    wav_with(
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
        frames,
    )
}

fn wav_with(spec: hound::WavSpec, frames: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut out, spec).unwrap();
    for i in 0..frames * u32::from(spec.channels) {
        let v = i16::try_from(i % 2000).unwrap() - 1000;
        match spec.sample_format {
            hound::SampleFormat::Int => writer.write_sample(v).unwrap(),
            hound::SampleFormat::Float => writer.write_sample(f32::from(v) / 32768.0).unwrap(),
        }
    }
    writer.finalize().unwrap();
    out.into_inner()
}

/// `wav` with a `LIST`/`INFO` chunk appended and the RIFF size fixed.
fn with_info(wav: &[u8], text: &[u8]) -> Vec<u8> {
    let mut info = b"INFOISFT".to_vec();
    info.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
    info.extend_from_slice(text);
    let mut out = wav.to_vec();
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&u32::try_from(info.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&info);
    let riff = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn a_float_wav_becomes_a_pcm16_wav_without_its_tags() {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let source = with_info(&wav_with(spec, 4410), b"leaked-home-path\0");
    assert!(contains(&source, b"leaked-home-path"));
    let out = run(&[Entry::file("sounds/ring.wav", &source)]).unwrap();
    let canonical = &out["sounds/ring.wav"];
    assert!(
        !contains(canonical, b"leaked-home-path"),
        "the tag survived"
    );
    assert!(!contains(canonical, b"LIST"));
    let reader = hound::WavReader::new(Cursor::new(canonical)).unwrap();
    let got = reader.spec();
    assert_eq!(
        (
            got.channels,
            got.sample_rate,
            got.bits_per_sample,
            got.sample_format
        ),
        (2, 44_100, 16, hound::SampleFormat::Int)
    );
    assert_eq!(reader.duration(), 4410);
}

#[test]
fn a_cue_at_the_limits_is_taken() {
    let at_limit = wav(MAX_SAMPLE_RATE, 2, MAX_SAMPLE_RATE * MAX_SOUND_SECONDS);
    let out = run(&[Entry::file("sounds/long.wav", &at_limit)]).unwrap();
    let reader = hound::WavReader::new(Cursor::new(&out["sounds/long.wav"])).unwrap();
    assert_eq!(reader.duration(), MAX_SAMPLE_RATE * MAX_SOUND_SECONDS);
}

#[test]
fn a_cue_over_a_limit_is_refused() {
    let long = wav(8000, 1, 8000 * MAX_SOUND_SECONDS + 1);
    assert_eq!(
        refused(&[Entry::file("sounds/long.wav", &long)]),
        SkinRule::SoundTooLong {
            max_seconds: MAX_SOUND_SECONDS
        }
    );
    assert_eq!(
        refused(&[Entry::file("sounds/hi.wav", &wav(96_000, 1, 96))]),
        SkinRule::SampleRateTooHigh {
            rate: 96_000,
            max: MAX_SAMPLE_RATE
        }
    );
    assert_eq!(
        refused(&[Entry::file("sounds/wide.wav", &wav(8000, 3, 80))]),
        SkinRule::TooManyChannels {
            channels: 3,
            max: MAX_SOUND_CHANNELS
        }
    );
}

#[test]
fn too_many_cues_are_refused() {
    let cue = wav(8000, 1, 8);
    let entries: Vec<_> = (0..=MAX_SOUND_CUES)
        .map(|i| Entry::file(&format!("sounds/{i:02}.wav"), &cue))
        .collect();
    assert_eq!(
        refused(&entries),
        SkinRule::TooManySoundCues {
            count: MAX_SOUND_CUES + 1,
            max: MAX_SOUND_CUES
        }
    );
}

#[test]
fn a_cue_of_another_type_than_declared_is_refused() {
    let rule = refused(&[Entry::file("sounds/ring.ogg", &wav(8000, 1, 8))]);
    assert!(matches!(rule, SkinRule::SoundDecode { .. }), "{rule:?}");
    let rule = refused(&[Entry::file("sounds/ring.flac", &wav(8000, 1, 8))]);
    assert!(matches!(rule, SkinRule::SoundDecode { .. }), "{rule:?}");
    let rule = refused(&[Entry::file("sounds/ring.mp3", b"ID3\x04")]);
    assert!(matches!(rule, SkinRule::UnsupportedEntryType { .. }));
}

#[test]
fn manifest_sounds_name_held_files_under_sounds() {
    let mut m = common::manifest();
    m["sounds"] = json!({ "message.received": "assets/ding.wav" });
    let err = run_with(&m, &[]).unwrap_err();
    assert_eq!(err.entry, "manifest.json");
    assert!(matches!(err.rule, SkinRule::BadValue { .. }));
    m["sounds"] = json!({ "message.received": "sounds/ding.wav" });
    let err = run_with(&m, &[]).unwrap_err();
    assert_eq!(err.entry, "sounds/ding.wav");
    assert!(matches!(err.rule, SkinRule::MissingEntry { .. }));
}

/// CRC-8, polynomial 0x07 (FLAC frame header).
fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |mut crc, byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x07
            };
        }
        crc
    })
}

/// CRC-16, polynomial 0x8005 (FLAC frame footer).
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |mut crc, byte| {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x8005
            };
        }
        crc
    })
}

/// A mono 8 kHz 16-bit FLAC of `frames` frames of 4096 constant samples,
/// whose STREAMINFO leaves the total sample count unknown (0, RFC 9639
/// §8.2): only counting the decoded frames bounds its length.
fn flac(frames: u8) -> Vec<u8> {
    let mut out = b"fLaC".to_vec();
    out.extend_from_slice(&[0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
    out.extend_from_slice(&4096u16.to_be_bytes());
    out.extend_from_slice(&4096u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]); // frame sizes unknown
    let packed: u64 = (8000 << 44) | (15 << 36); // rate, mono, 16 bits, total 0
    out.extend_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&[0; 16]); // MD5 unknown
    for number in 0..frames {
        // Sync + fixed blocking; 4096 samples, 8 kHz; mono, 16 bits; number.
        let mut frame = vec![0xff, 0xf8, 0xc4, 0x08, number];
        frame.push(crc8(&frame));
        frame.extend_from_slice(&[0x00, 0x01, 0x00]); // CONSTANT subframe, 256
        let footer = crc16(&frame);
        frame.extend_from_slice(&footer.to_be_bytes());
        out.extend_from_slice(&frame);
    }
    out
}

#[test]
fn a_flac_of_unknown_length_is_bounded_by_counting_its_frames() {
    let out = run(&[Entry::file("sounds/ok.flac", &flac(2))]).unwrap();
    let reader = hound::WavReader::new(Cursor::new(&out["sounds/ok.flac"])).unwrap();
    assert_eq!((reader.spec().sample_rate, reader.duration()), (8000, 8192));
    // Ten frames are 40 960 samples, past 5 s at 8 kHz.
    assert_eq!(
        refused(&[Entry::file("sounds/long.flac", &flac(10))]),
        SkinRule::SoundTooLong {
            max_seconds: MAX_SOUND_SECONDS
        }
    );
}
