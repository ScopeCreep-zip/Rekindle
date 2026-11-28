use super::*;

const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";

#[test]
fn hex_keys_decode_to_bytes() {
    let bytes = public_key_hex(HEX64).unwrap().to_bytes();
    assert_eq!(bytes[..3], [0x3f, 0x1a, 0x9c]);
    assert_eq!(bytes[31], 0x1a);
    assert_eq!(pseudonym_hex(HEX64).unwrap().to_bytes(), bytes);
}

#[test]
fn record_keys() {
    assert!(record_key(&format!("VLD0:{B64}")).is_ok());
    assert!(record_key(&format!("VLD0:{B64}:{B64}")).is_ok());
    assert_eq!(record_key(""), Err(KeyFormatError::Empty));
    assert_eq!(
        record_key(&format!("XXX0:{B64}")),
        Err(KeyFormatError::BadPrefix)
    );
    assert_eq!(
        record_key(&format!("VLD0:{B64}:{B64}:{B64}")),
        Err(KeyFormatError::TooManySegments)
    );
    assert!(matches!(
        record_key(&format!("VLD0:{}", &B64[..42])),
        Err(KeyFormatError::BadLength {
            expected: 43,
            got: 42
        })
    ));
    assert!(matches!(
        record_key(&format!("VLD0:{B64}x")),
        Err(KeyFormatError::BadLength { got: 44, .. })
    ));
    let bad = format!("VLD0:{}/", &B64[..42]);
    assert!(matches!(
        record_key(&bad),
        Err(KeyFormatError::BadChar { index: 47 })
    ));
    assert!(record_key(&format!("VLD0:{}\0", &B64[..42])).is_err());
}

#[test]
fn hex_ids() {
    assert!(public_key_hex(HEX64).is_ok());
    assert!(pseudonym_hex(HEX64).is_ok());
    assert!(public_key_hex(&HEX64.to_uppercase()).is_err());
    assert!(public_key_hex(&HEX64[..63]).is_err());
    assert!(call_id(&HEX64[..32]).is_ok());
    assert!(hex16_id(&HEX64[..32]).is_ok());
    assert!(call_id(&"é".repeat(16)).is_err());
    assert_eq!(call_id(""), Err(KeyFormatError::Empty));
    assert!(call_id("6f9619ff-8b86-d011-b42d-00cf4fc964ff").is_err());
}

#[test]
fn names_and_community_refs() {
    assert_eq!(name("  Gamers  ", 100), Ok("Gamers"));
    assert_eq!(name("   ", 100), Err(KeyFormatError::Empty));
    assert!(matches!(
        name(&"a".repeat(101), 100),
        Err(KeyFormatError::TooLong { .. })
    ));
    assert!(name("bad\u{7}name", 100).is_err());
    assert_eq!(
        community_ref(&format!("VLD0:{B64}")),
        Ok(CommunityRef::Key(RecordKeyStr(format!("VLD0:{B64}"))))
    );
    assert_eq!(
        community_ref("Gamers"),
        Ok(CommunityRef::Name("Gamers".into()))
    );
    assert!(community_ref("VLD0:short").is_err());
}

#[test]
fn text_counts_chars_and_allows_newlines() {
    assert_eq!(text("", 4), Ok(""));
    assert_eq!(text("a\nb\tc", 5), Ok("a\nb\tc"));
    assert!(text(&"é".repeat(4), 4).is_ok());
    assert_eq!(
        text(&"é".repeat(5), 4),
        Err(KeyFormatError::TooLong { max: 4, got: 5 })
    );
    assert_eq!(text("ab\x1b", 8), Err(KeyFormatError::BadChar { index: 2 }));
}

/// Bidi overrides and other hidden format characters are refused in
/// names and text alike; the emoji joiner is not.
#[test]
fn hidden_format_characters_are_refused() {
    assert_eq!(
        name("ann\u{202E}gpj.exe", 100),
        Err(KeyFormatError::BadChar { index: 3 })
    );
    assert_eq!(
        text("hi\u{2066}x", 100),
        Err(KeyFormatError::BadChar { index: 2 })
    );
    assert!(text("a\u{200B}b", 100).is_err());
    assert!(name("\u{FEFF}Bob", 100).is_err());
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(text(family, 100), Ok(family));
    assert_eq!(name(family, 100), Ok(family));
}

#[test]
fn a_message_body_is_bounded_text_and_not_blank() {
    assert_eq!(message_body("hi"), Ok("hi"));
    assert_eq!(message_body(" \n "), Err(KeyFormatError::Empty));
    assert!(message_body(&"x".repeat(MAX_MESSAGE_LEN + 1)).is_err());
    assert!(message_body("a\u{202E}b").is_err());
}

#[test]
fn channel_refs() {
    let id = "0123456789abcdef0123456789abcdef";
    assert!(matches!(channel_ref(id), Ok(ChannelRef::Id(_))));
    assert_eq!(
        channel_ref(" general "),
        Ok(ChannelRef::Name("general".into()))
    );
    assert!(channel_ref("").is_err());
}
