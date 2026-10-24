use super::*;

/// A tone at `hz`, `ms` long, at the input rate.
fn tone(hz: f32, ms: u32) -> Vec<f32> {
    let n = INPUT_SAMPLE_RATE * ms / 1000;
    let step = 2.0 * std::f32::consts::PI * hz / 16_000.0;
    let mut phase = 0f32;
    (0..n)
        .map(|_| {
            let sample = 0.5 * phase.sin();
            phase += step;
            sample
        })
        .collect()
}

#[test]
fn a_recording_becomes_ogg_opus_with_empty_tags_and_its_true_length() {
    // Whatever the webview recorded (WebM/Opus in Chromium-based views,
    // MP4/AAC in WKWebView), it hands over decoded PCM; the file that goes
    // out is Ogg Opus.
    let pcm = tone(440.0, 2_340);
    let bytes = pcm
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect::<Vec<u8>>();
    let ogg = encode(&pcm_from_le_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(&ogg[..4], b"OggS");
    assert_eq!(ogg[5] & 0x02, 0x02, "the first page is marked first");
    let head = &ogg[28..47];
    assert_eq!(&head[..8], b"OpusHead");
    let tags_at = ogg.windows(8).position(|w| w == b"OpusTags").unwrap();
    assert_eq!(
        &ogg[tags_at..tags_at + 16],
        EMPTY_TAGS,
        "no vendor string, no comments"
    );
    assert!(
        !ogg.windows(6).any(|w| w == b"libopu"),
        "no encoder vendor anywhere"
    );
    let info = analyze(&ogg).unwrap();
    assert_eq!(info.duration_ms, 2_340);
    assert_eq!(info.waveform.len(), WAVEFORM_PEAKS);
    assert!(info.waveform.iter().filter(|&&p| p > 32).count() > WAVEFORM_PEAKS / 2);
}

#[test]
fn the_duration_comes_from_the_audio_not_the_sender() {
    // A sender may claim anything; the receiver reads the granule position.
    let ogg = encode(&tone(300.0, 1_000)).unwrap();
    assert_eq!(analyze(&ogg).unwrap().duration_ms, 1_000);
    let silent = encode(&vec![0.0; usize::try_from(INPUT_SAMPLE_RATE).unwrap()]).unwrap();
    assert!(analyze(&silent).unwrap().waveform.iter().all(|&p| p < 4));
}

#[test]
fn bounds_and_foreign_formats_are_refused() {
    assert!(matches!(
        encode(&tone(440.0, 150)),
        Err(VoiceNoteError::Duration { .. })
    ));
    assert!(matches!(
        encode(&vec![
            0.0;
            usize::try_from(INPUT_SAMPLE_RATE * 301).unwrap()
        ]),
        Err(VoiceNoteError::Duration { .. })
    ));
    assert_eq!(pcm_from_le_bytes(&[0, 0, 0]), Err(VoiceNoteError::Pcm));
    // A WebM (EBML) file labelled as a voice message is refused.
    let webm = [0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01];
    assert!(matches!(analyze(&webm), Err(VoiceNoteError::Format(_))));
    // One flipped byte breaks a page CRC.
    let mut ogg = encode(&tone(440.0, 500)).unwrap();
    let last = ogg.len() - 1;
    ogg[last] ^= 0xff;
    assert!(matches!(analyze(&ogg), Err(VoiceNoteError::Format(_))));
}

#[test]
fn a_non_empty_opus_tags_is_refused() {
    let ogg = encode(&tone(440.0, 500)).unwrap();
    let packets = ogg::read(&ogg).unwrap();
    let mut writer = ogg::OggWriter::new(1);
    writer.page(&[&packets[0].data], 0, true, false);
    writer.page(&[b"OpusTags\x07\0\0\0libopus\0\0\0\0"], 0, false, false);
    let audio: Vec<&[u8]> = packets[2..].iter().map(|p| p.data.as_slice()).collect();
    let end = packets.last().unwrap().page_granule.unwrap();
    writer.page(&audio, end, false, true);
    let tagged = writer.finish();
    assert!(matches!(analyze(&tagged), Err(VoiceNoteError::Format(_))));
}
