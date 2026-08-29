use super::*;

#[test]
fn name_validation_length_bounds() {
    assert!(validate_expression_name("ab").is_ok());
    assert!(validate_expression_name(&"x".repeat(32)).is_ok());
    assert!(validate_expression_name("a").is_err());
    assert!(validate_expression_name(&"x".repeat(33)).is_err());
}

#[test]
fn name_validation_charset() {
    assert!(validate_expression_name("abc_123").is_ok());
    assert!(validate_expression_name("hello-world").is_err());
    assert!(validate_expression_name("hello world").is_err());
    assert!(validate_expression_name("héllo").is_err());
}

#[test]
fn normalize_tags_trims_lowercases_dedupes() {
    let out = normalize_tags(vec![
        "  Foo ".into(),
        "FOO".into(),
        "bar-baz".into(),
        String::new(),
    ])
    .unwrap();
    assert_eq!(out, vec!["foo", "bar-baz"]);
}

#[test]
fn normalize_tags_rejects_overlimits() {
    assert!(normalize_tags(vec!["x".into(); 17]).is_err());
    assert!(normalize_tags(vec!["x".repeat(25)]).is_err());
    assert!(normalize_tags(vec!["bad!tag".into()]).is_err());
}

#[test]
fn detect_png_magic() {
    let png = b"\x89PNG\r\n\x1a\n....";
    assert_eq!(detect_image_media_type(png, false), Some("image/png"));
}

#[test]
fn detect_webp_magic() {
    let webp = b"RIFF....WEBPmore";
    assert_eq!(detect_image_media_type(webp, false), Some("image/webp"));
}

#[test]
fn detect_gif_only_when_animated_allowed() {
    let gif = b"GIF89a....";
    assert_eq!(detect_image_media_type(gif, true), Some("image/gif"));
    assert_eq!(detect_image_media_type(gif, false), None);
}

#[test]
fn detect_audio_ogg_webm_mp3() {
    assert_eq!(detect_audio_kind(b"OggS...."), Some("audio/ogg"));
    assert_eq!(
        detect_audio_kind(b"\x1A\x45\xDF\xA3...."),
        Some("audio/webm")
    );
    assert_eq!(detect_audio_kind(b"ID3...."), Some("audio/mpeg"));
    // MP3 sync word
    assert_eq!(
        detect_audio_kind(&[0xFF, 0xFB, 0x90, 0x00]),
        Some("audio/mpeg")
    );
    assert_eq!(detect_audio_kind(b"random"), None);
}

#[test]
fn validate_emoji_rejects_empty_and_oversize() {
    assert!(validate_emoji_bytes(&[], false).is_err());
    let oversize = vec![0u8; MAX_STATIC_EMOJI_BYTES + 1];
    assert!(matches!(
        validate_emoji_bytes(&oversize, false),
        Err(ChannelError::BodyTooLarge { .. })
    ));
}

#[test]
fn validate_emoji_accepts_png() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0u8; 100]);
    assert!(validate_emoji_bytes(&png, false).is_ok());
}

#[test]
fn validate_sticker_animated_rejects_static_gif_unless_allowed() {
    let gif = b"GIF89a....".to_vec();
    assert!(validate_sticker_bytes(&gif, true).is_ok());
    assert!(validate_sticker_bytes(&gif, false).is_err());
}

#[test]
fn validate_soundboard_recognises_ogg() {
    let mut ogg = b"OggS".to_vec();
    ogg.extend_from_slice(&[0u8; 100]);
    assert!(validate_soundboard_bytes(&ogg).is_ok());
    assert!(validate_soundboard_bytes(b"raw bytes").is_err());
}
