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
fn validate_soundboard_recognises_ogg() {
    let mut ogg = b"OggS".to_vec();
    ogg.extend_from_slice(&[0u8; 100]);
    assert!(validate_soundboard_bytes(&ogg).is_ok());
    assert!(validate_soundboard_bytes(b"raw bytes").is_err());
}
