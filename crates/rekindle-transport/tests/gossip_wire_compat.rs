//! Cross-track gossip wire compatibility.
//!
//! The two shells were gossiping in formats neither could read — the
//! desktop a Cap'n Proto `CommunityEnvelope` inside a `SignedEnvelope`
//! straight on `app_message`, the daemon a postcard `GossipPayload`
//! inside a `TypeId::GossipBroadcast` frame. PATH 2 of the three-path
//! model worked in neither direction between tracks, and nothing failed
//! loudly: each side simply dropped what the other sent.
//!
//! These pin the shared format. They are wire tests on purpose — a
//! compile pass proves nothing here, because both encodings type-check
//! perfectly well on their own side.

use rekindle_protocol::capnp_envelope::{
    decode_signed_envelope, encode_community_envelope, encode_signed_envelope,
    try_decode_community_envelope,
};
use rekindle_protocol::dht::community::envelope::{
    sign_envelope, verify_envelope, CommunityEnvelope,
};
use rekindle_secrets::ed25519_dalek::SigningKey;

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The pseudonym that goes in the envelope must be the public half of
/// the key that signs it — `verify_envelope` resolves the verifying key
/// from this field, so a mismatch is exactly the forgery it exists to
/// reject. (It rejected my first draft of these tests, which used a
/// made-up hex string here.)
fn my_pseudonym() -> String {
    hex::encode(signing_key().verifying_key().to_bytes())
}

fn notification() -> CommunityEnvelope {
    CommunityEnvelope::MessageNotification {
        channel_id: "ab".repeat(8),
        message_id: "msg_0123".into(),
        author_pseudonym: "cd".repeat(32),
        subkey_index: 42,
        lamport_ts: 9,
        sequence: 0,
        content_hash: "ef".repeat(32),
        timestamp: 1_700_000_000,
    }
}

/// The full path a broadcast takes: encode → sign → encode envelope →
/// (wire) → decode envelope → verify → decode inner.
///
/// This is exactly what `mesh_broadcast::send_to_mesh` does on the way
/// out and `dispatch_bare_gossip` does on the way in, so a break in
/// either direction shows up here.
#[test]
fn a_broadcast_round_trips_through_the_shared_format() {
    let key = signing_key();
    let envelope_bytes = encode_community_envelope(&notification()).expect("encode inner");
    let signed = sign_envelope(&key, "gov-key", &my_pseudonym(), &envelope_bytes);
    let wire = encode_signed_envelope(&signed);

    let received = decode_signed_envelope(&wire).expect("decode envelope");
    verify_envelope(&received).expect("signature must verify");

    let inner = try_decode_community_envelope(&received.envelope_bytes)
        .expect("decode inner")
        .expect("known variant");
    // Compared by re-encoding rather than `PartialEq`, which the
    // envelope does not derive — and this is the stronger check: it
    // pins the bytes, not just the fields we happened to look at.
    assert_eq!(
        encode_community_envelope(&inner).expect("re-encode"),
        envelope_bytes,
    );
}

/// A tampered payload must not verify.
///
/// The unframed inbound path has no transport-layer authentication, so
/// this signature is the only thing between a forged `sender_pseudonym`
/// and the handler.
#[test]
fn a_tampered_payload_fails_verification() {
    let key = signing_key();
    let envelope_bytes = encode_community_envelope(&notification()).expect("encode");
    let mut signed = sign_envelope(&key, "gov-key", &my_pseudonym(), &envelope_bytes);

    signed.envelope_bytes[4] ^= 0xff;

    assert!(
        verify_envelope(&signed).is_err(),
        "a modified payload must not verify under the original signature"
    );
}

/// `WatchRelay` survives the round trip.
///
/// Mutual Aid §14.3 exists only in `CommunityEnvelope` — the daemon's
/// old three-variant `GossipPayload` could not express it at all, which
/// is half the reason the relay was never implemented on that track.
#[test]
fn watch_relay_crosses_the_wire() {
    let key = signing_key();
    let envelope = CommunityEnvelope::WatchRelay {
        record_key: "VLD0:abcdef".into(),
        subkey: 17,
        content_hash: "ab".repeat(32),
        observer_pseudonym: "cd".repeat(32),
    };

    let envelope_bytes = encode_community_envelope(&envelope).expect("encode");
    let signed = sign_envelope(&key, "gov-key", &my_pseudonym(), &envelope_bytes);
    let wire = encode_signed_envelope(&signed);

    let received = decode_signed_envelope(&wire).expect("decode");
    verify_envelope(&received).expect("verify");
    let inner = try_decode_community_envelope(&received.envelope_bytes)
        .expect("decode inner")
        .expect("known variant");

    assert_eq!(
        encode_community_envelope(&inner).expect("re-encode"),
        envelope_bytes,
    );
}

/// The relay carries a hash and never the value.
///
/// Gossip is unencrypted at the envelope layer, so a relay that shipped
/// the bytes would hand a channel message's ciphertext to every
/// forwarding hop. Asserted structurally rather than trusted to review.
#[test]
fn watch_relay_carries_no_payload_bytes() {
    let secret = b"the-ciphertext-nobody-should-see";
    let envelope = CommunityEnvelope::WatchRelay {
        record_key: "VLD0:abcdef".into(),
        subkey: 17,
        content_hash: blake3::hash(secret).to_hex().to_string(),
        observer_pseudonym: "cd".repeat(32),
    };

    let encoded = encode_community_envelope(&envelope).expect("encode");
    assert!(
        !encoded.windows(secret.len()).any(|window| window == secret),
        "a WatchRelay must not carry the value it announces"
    );
}

/// A directed payload is never gossip-forwarded.
///
/// §10.6 and the join path both depend on it: a `JoinAccepted` is
/// wrapped for one recipient, and amplifying it to the whole mesh
/// within the TTL benefits nobody. Ported from a unit test that asserted
/// the same thing about the deleted postcard envelope — where the rule
/// had drifted into two disagreeing copies, one listing six variants and
/// one four.
#[test]
fn directed_payloads_are_not_forwarded() {
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    let join_accepted = CommunityEnvelope::Control(ControlPayload::JoinAccepted {
        mek_encrypted: vec![],
        mek_generation: 0,
        members: Vec::new(),
        member_registry_key: None,
        slot_index: None,
        wrapped_slot_seed: None,
    });
    assert!(join_accepted.is_directed());

    // Both halves of the drift, now covered by the one definition.
    assert!(CommunityEnvelope::Control(ControlPayload::JoinRejected {
        reason: "no".into()
    })
    .is_directed());
    assert!(CommunityEnvelope::Control(ControlPayload::KickedNotification).is_directed());

    // A channel message notification is exactly what *must* be
    // forwarded — the epidemic wave is how it reaches members outside
    // the sender's fan-out.
    assert!(!notification().is_directed());
}

/// The notification carries the manifest, not the cargo.
///
/// Gossip is unencrypted at the envelope layer and fans out to
/// `min(N, 6)` peers per hop for five hops, so payload size multiplies.
/// A notification names the message; the body stays in the record.
#[test]
fn message_notification_stays_compact() {
    let bytes = encode_community_envelope(&notification()).expect("encode");
    assert!(
        bytes.len() < 400,
        "MessageNotification should stay compact, was {} bytes",
        bytes.len()
    );
}
