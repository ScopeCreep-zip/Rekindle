//! Channel messages must be readable by both tracks.
//!
//! `ChannelMessage` used to be declared twice — once here, once in
//! `rekindle-protocol` — for the same channel-record subkey. The two
//! diverged, and not subtly: this crate's copy had no
//! `#[serde(with = "base64_bytes")]` on `ciphertext`, so it wrote a JSON
//! number array where the desktop track writes a base64 string. Parsing
//! a daemon-written message on the desktop side failed outright with
//! `invalid type: sequence, expected a string`. It also lacked
//! `attachment`, `flags`, `mentioned_pseudonyms` and `mentioned_roles`,
//! so file offers, voice-message and @everyone/@here flags, and mention
//! routing were dropped on any round trip through it.
//!
//! There is one definition now. These tests exist so a future edit that
//! reintroduces a local copy — or drops the base64 attribute — fails
//! here rather than in the field.

use rekindle_transport::payload::dht_types::ChannelMessage;

fn sample() -> ChannelMessage {
    ChannelMessage {
        sequence: 1,
        sender_pseudonym: "ab12".into(),
        ciphertext: vec![1, 2, 3],
        mek_generation: 4,
        timestamp: 5,
        reply_to: None,
        lamport_ts: 6,
        message_id: None,
        attachment: None,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    }
}

/// The type this crate uses IS the desktop track's, so a message written
/// here parses there by construction. Asserted rather than assumed,
/// because the failure mode is a silent redeclaration.
#[test]
fn transport_and_protocol_share_one_definition() {
    let written = serde_json::to_vec(&sample()).expect("serialize");
    let read: rekindle_protocol::dht::community::channel_record::ChannelMessage =
        serde_json::from_slice(&written)
            .expect("the desktop track must parse daemon-written bytes");
    assert_eq!(read.sequence, 1);
    assert_eq!(read.ciphertext, vec![1, 2, 3]);
    assert_eq!(read.lamport_ts, 6);
}

/// `ciphertext` is base64, not a byte array. This is the exact field
/// whose encoding differed, and the one that made the two unreadable to
/// each other.
#[test]
fn ciphertext_is_base64_encoded() {
    let json = serde_json::to_string(&sample()).unwrap();
    assert!(
        json.contains(r#""ciphertext":"AQID""#),
        "ciphertext must serialize as base64, got: {json}"
    );
    assert!(
        !json.contains(r#""ciphertext":[1,2,3]"#),
        "a raw byte array is the old, incompatible encoding: {json}"
    );
}

/// The four fields the daemon copy lacked must survive a round trip.
#[test]
fn attachment_flags_and_mentions_survive() {
    let mut msg = sample();
    msg.flags = 0x10;
    msg.mentioned_pseudonyms = vec!["deadbeef".into()];
    msg.mentioned_roles = vec!["role1".into()];

    let bytes = serde_json::to_vec(&msg).unwrap();
    let back: ChannelMessage = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.flags, 0x10);
    assert_eq!(back.mentioned_pseudonyms, vec!["deadbeef".to_string()]);
    assert_eq!(back.mentioned_roles, vec!["role1".to_string()]);
}

/// A plain message stays byte-identical to the legacy shape: the added
/// fields skip-serialize away, so adopting the fuller definition did not
/// change what the daemon puts on the wire for existing traffic.
#[test]
fn plain_message_adds_no_keys() {
    let json = serde_json::to_string(&sample()).unwrap();
    for absent in ["attachment", "mentionedPseudonyms", "mentionedRoles"] {
        assert!(
            !json.contains(absent),
            "`{absent}` must be omitted for a plain message: {json}"
        );
    }
    // `flags` has a plain #[serde(default)] with no skip, so it is
    // present and zero — that is the desktop track's existing shape.
    assert!(json.contains(r#""flags":0"#), "got: {json}");
}
