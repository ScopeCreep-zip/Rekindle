use super::*;

const AUTHOR: [u8; 32] = [9; 32];
const OTHER: [u8; 32] = [8; 32];
const AT: BodyPosition<'static> = BodyPosition {
    anchor: "VLD0:anchor",
    channel_id: "chan-a",
    author: &AUTHOR,
    stream_seq: 42,
};

#[test]
fn round_trip() {
    let ct = encrypt_channel_body(&[1u8; 32], AT, b"hello").unwrap();
    assert_eq!(decrypt_channel_body(&[1u8; 32], AT, &ct).unwrap(), b"hello");
}

/// A body re-placed under another community, channel, author or
/// stream position does not open.
#[test]
fn bound_to_its_position() {
    let ct = encrypt_channel_body(&[1u8; 32], AT, b"hello").unwrap();
    for moved in [
        BodyPosition {
            anchor: "VLD0:other",
            ..AT
        },
        BodyPosition {
            channel_id: "chan-b",
            ..AT
        },
        BodyPosition {
            author: &OTHER,
            ..AT
        },
        BodyPosition {
            stream_seq: 43,
            ..AT
        },
    ] {
        assert!(decrypt_channel_body(&[1u8; 32], moved, &ct).is_err());
    }
}

/// A tag is fixed by the key and the stream: another generation's key
/// or another channel gives another tag.
#[test]
fn a_tag_is_bound_to_its_key_and_stream() {
    let tag = channel_tag(&[1u8; 32], "chan-a");
    assert_eq!(tag, channel_tag(&[1u8; 32], "chan-a"));
    assert_ne!(tag, channel_tag(&[2u8; 32], "chan-a"));
    assert_ne!(tag, channel_tag(&[1u8; 32], "chan-b"));
}

/// A body encrypted without AAD (the old path) does not decrypt.
#[test]
fn no_aadless_bodies() {
    let legacy = MediaEncryptionKey::from_bytes([1u8; 32], 0)
        .encrypt(b"hello")
        .unwrap();
    assert!(decrypt_channel_body(&[1u8; 32], AT, &legacy).is_err());
}
