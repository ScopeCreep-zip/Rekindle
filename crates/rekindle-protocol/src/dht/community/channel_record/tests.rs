use super::{
    decode_channel_entries, ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollCreate,
    ChannelPollVote, ChannelReaction, ChannelRecordEntry, ChannelSubkeyPayload,
};
use ed25519_dalek::SigningKey;
use rekindle_secrets::derive::sign_with_pseudonym;
use rekindle_types::id::PseudonymKey;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42u8; 32])
}

fn test_author_hex() -> String {
    hex::encode(test_signing_key().verifying_key().to_bytes())
}

fn sample_message() -> ChannelMessage {
    ChannelMessage {
        sequence: 7,
        sender_pseudonym: test_author_hex(),
        ciphertext: vec![1, 2, 3],
        mek_generation: 2,
        timestamp: 1234,
        reply_to: None,
        lamport_ts: 9,
        message_id: Some("msg-1".into()),
        attachment: None,
        flags: 0,
        mentioned_pseudonyms: Vec::new(),
        mentioned_roles: Vec::new(),
    }
}

fn signed_payload_bytes(entries: Vec<ChannelRecordEntry>) -> (PseudonymKey, Vec<u8>) {
    let signing = test_signing_key();
    let pseudo = PseudonymKey(signing.verifying_key().to_bytes());
    let mut payload = ChannelSubkeyPayload {
        author_pseudonym: pseudo.clone(),
        entries,
        signature: Vec::new(),
    };
    let sig = sign_with_pseudonym(&signing, &payload.signing_bytes());
    payload.signature = sig.to_vec();
    let bytes = serde_json::to_vec(&payload).expect("serialize signed payload");
    (pseudo, bytes)
}

#[test]
fn decode_signed_message_payload() {
    let (_pseudo, page) = signed_payload_bytes(vec![ChannelRecordEntry::Message(sample_message())]);
    let entries = decode_channel_entries(&page).unwrap();
    assert!(matches!(&entries[0], ChannelRecordEntry::Message(message) if message.sequence == 7));
}

#[test]
fn decode_signed_mixed_entry_page() {
    let entries = vec![
        ChannelRecordEntry::Message(sample_message()),
        ChannelRecordEntry::Reaction(ChannelReaction {
            message_id: "msg-1".into(),
            expression: "🔥".into(),
            added: true,
            lamport: 10,
        }),
        ChannelRecordEntry::PollCreate(ChannelPollCreate {
            poll_id: [4u8; 16],
            message_id: "msg-1".into(),
            question: "Ready?".into(),
            answers: vec!["Yes".into(), "No".into()],
            multi_select: false,
            expires_at: None,
            lamport: 11,
        }),
        ChannelRecordEntry::PollVote(ChannelPollVote {
            poll_id: [4u8; 16],
            selected_answers: vec![0],
            lamport: 12,
        }),
        ChannelRecordEntry::HandRaise(ChannelHandRaise {
            raised: true,
            lamport: 13,
        }),
        ChannelRecordEntry::Forward(ChannelForward {
            sequence: 8,
            sender_pseudonym: test_author_hex(),
            original_message_id: "msg-source".into(),
            original_channel_id: "deadbeefdeadbeefdeadbeefdeadbeef".into(),
            original_author: "ff".repeat(32),
            content_snapshot: vec![9, 9, 9],
            mek_generation: 4,
            timestamp: 5678,
            lamport_ts: 14,
            message_id: Some("msg-fwd-1".into()),
        }),
    ];
    let (_pseudo, page) = signed_payload_bytes(entries);
    let decoded = decode_channel_entries(&page).unwrap();
    assert_eq!(decoded.len(), 6);
    assert!(matches!(
        &decoded[1],
        ChannelRecordEntry::Reaction(reaction)
            if reaction.expression == "🔥" && reaction.added
    ));
    assert!(matches!(
        &decoded[2],
        ChannelRecordEntry::PollCreate(create)
            if create.question == "Ready?" && create.answers.len() == 2
    ));
    assert!(matches!(
        &decoded[3],
        ChannelRecordEntry::PollVote(vote)
            if vote.selected_answers == vec![0]
    ));
    assert!(matches!(
        &decoded[4],
        ChannelRecordEntry::HandRaise(hand_raise) if hand_raise.raised
    ));
    assert!(matches!(
        &decoded[5],
        ChannelRecordEntry::Forward(fwd) if fwd.original_message_id == "msg-source" && fwd.sequence == 8
    ));
}

#[test]
fn decode_rejects_unsigned_legacy_format() {
    // Architecture §26 W26 — old wire format (bare `Vec<ChannelRecordEntry>`)
    // is no longer accepted. The new format is the signed wrapper.
    let bytes = serde_json::to_vec(&vec![ChannelRecordEntry::Message(sample_message())]).unwrap();
    assert!(decode_channel_entries(&bytes).is_err());
}

#[test]
fn decode_rejects_forged_sender_pseudonym_in_message() {
    // Architecture §26 W26 — author X writes a message claiming
    // sender_pseudonym = victim. Wrapper signature is valid, but the
    // per-entry sender doesn't match the wrapper author. Decode must
    // reject so the receiver doesn't attribute the forgery to victim.
    let signing = test_signing_key();
    let pseudo = PseudonymKey(signing.verifying_key().to_bytes());
    let mut forged = sample_message();
    forged.sender_pseudonym = "ff".repeat(32); // claims to be someone else
    let mut payload = ChannelSubkeyPayload {
        author_pseudonym: pseudo,
        entries: vec![ChannelRecordEntry::Message(forged)],
        signature: Vec::new(),
    };
    let sig = sign_with_pseudonym(&signing, &payload.signing_bytes());
    payload.signature = sig.to_vec();
    let bytes = serde_json::to_vec(&payload).unwrap();
    assert!(decode_channel_entries(&bytes).is_err());
}

#[test]
fn decode_rejects_tampered_signature() {
    // Build a valid signed payload, then forge it by replacing the
    // signature with one made by a different key. Verification must fail.
    let signing = SigningKey::from_bytes(&[0x42u8; 32]);
    let pseudo = PseudonymKey(signing.verifying_key().to_bytes());
    let entries = vec![ChannelRecordEntry::Message(sample_message())];
    let mut payload = ChannelSubkeyPayload {
        author_pseudonym: pseudo,
        entries,
        signature: Vec::new(),
    };
    // Sign with a DIFFERENT key — the verify check should reject.
    let attacker = SigningKey::from_bytes(&[0x99u8; 32]);
    let bogus_sig = sign_with_pseudonym(&attacker, &payload.signing_bytes());
    payload.signature = bogus_sig.to_vec();
    let bytes = serde_json::to_vec(&payload).unwrap();
    assert!(decode_channel_entries(&bytes).is_err());
}
