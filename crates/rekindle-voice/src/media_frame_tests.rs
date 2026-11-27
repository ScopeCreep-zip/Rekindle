use super::*;

#[test]
fn sequenced_round_trip() {
    let framed = sequenced(ENVELOPE_TAG, 0xDEAD_BEEF, b"abc");
    assert_eq!(
        split_sequenced(&framed),
        Some((ENVELOPE_TAG, 0xDEAD_BEEF, &b"abc"[..]))
    );
}

#[test]
fn padding_is_bound_to_its_sender_and_sequence() {
    let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
    let mut datagram = sequenced(PADDING_TAG, 41, &[0u8; PADDING_HEADER_LEN + 10]);
    assert!(sign_padding(&mut datagram, &key));
    let (_, seq, payload) = split_sequenced(&datagram).unwrap();
    assert_eq!(
        open_padding(seq, payload),
        Some(key.verifying_key().to_bytes())
    );
    assert_eq!(
        open_padding(42, payload),
        None,
        "replayed under another sequence"
    );
    let mut short = sequenced(PADDING_TAG, 1, &[0u8; 10]);
    assert!(!sign_padding(&mut short, &key));
    let mut voice = sequenced(VOICE_TAG, 1, &[0u8; PADDING_HEADER_LEN]);
    assert!(!sign_padding(&mut voice, &key));
}

/// The §6 tag ledger: the ingress tags (`'D'`, `'G'`) and the media
/// tags share the first byte, so no two may collide, and an ingress
/// datagram is never mistaken for media.
#[test]
fn media_and_ingress_tags_are_distinct() {
    use rekindle_codec::ingress::{DIRECT_TAG, GOSSIP_TAG};
    let tags = [
        VOICE_TAG,
        ENVELOPE_TAG,
        PADDING_TAG,
        FEEDBACK_TAG,
        crate::receiver_report::RECEIVER_REPORT_TAG,
        DIRECT_TAG,
        GOSSIP_TAG,
    ];
    for (i, a) in tags.iter().enumerate() {
        assert!(
            !tags[i + 1..].contains(a),
            "tag {} is reused",
            char::from(*a)
        );
    }
    assert_eq!(split_sequenced(&[DIRECT_TAG, 0, 0, 0, 0]), None);
    assert_eq!(split_sequenced(&[GOSSIP_TAG, 0, 0, 0, 0]), None);
}

#[test]
fn other_tags_and_short_frames_are_not_sequenced() {
    assert_eq!(split_sequenced(&[FEEDBACK_TAG, 0, 0, 0, 0]), None);
    assert_eq!(split_sequenced(&[VOICE_TAG, 1, 2]), None);
    assert_eq!(split_sequenced(&[]), None);
}
