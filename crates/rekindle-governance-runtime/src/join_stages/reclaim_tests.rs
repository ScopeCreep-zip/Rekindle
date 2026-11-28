use super::is_reclaimable;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;
use std::collections::HashSet;

/// A real, correctly-signed presence row for `signing`.
fn signed_row(signing: &SigningKey, heartbeat: u64) -> Vec<u8> {
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(signing.verifying_key().to_bytes()),
        status: "online".into(),
        last_heartbeat: heartbeat,
        ..Default::default()
    };
    let sig = derive::sign_with_pseudonym(signing, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    serde_json::to_vec(&presence).expect("serialize")
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn empty_and_garbage_rows_are_reclaimable() {
    let none = HashSet::new();
    assert!(is_reclaimable(b"", &none), "empty payload");
    assert!(is_reclaimable(b"not json at all", &none), "malformed");
    assert!(is_reclaimable(br#"{"bogus":1}"#, &none), "wrong shape");
}

/// The security-critical case. Under a shared slot seed anyone can
/// write any slot, so a row that fails W26 proves nothing about who
/// wrote it and cannot hold the slot against a real member.
#[test]
fn forged_signature_does_not_hold_a_slot() {
    let mut row: MemberPresence = MemberPresence {
        pseudonym_key: PseudonymKey(key(7).verifying_key().to_bytes()),
        status: "online".into(),
        last_heartbeat: 1_000,
        ..Default::default()
    };
    // Signed by somebody else entirely.
    let sig = derive::sign_with_pseudonym(&key(9), &row.signing_bytes());
    row.signature = sig.to_vec();
    let raw = serde_json::to_vec(&row).expect("serialize");
    assert!(is_reclaimable(&raw, &HashSet::new()));
}

#[test]
fn short_signature_does_not_hold_a_slot() {
    let mut row = MemberPresence {
        pseudonym_key: PseudonymKey(key(3).verifying_key().to_bytes()),
        ..Default::default()
    };
    row.signature = vec![0u8; 8];
    let raw = serde_json::to_vec(&row).expect("serialize");
    assert!(is_reclaimable(&raw, &HashSet::new()));
}

/// The MLS blank-on-Remove analogue, and the half that needs no new
/// wire format.
#[test]
fn banned_member_frees_the_slot() {
    let signing = key(5);
    let raw = signed_row(&signing, 1_000);
    let hex_key = hex::encode(signing.verifying_key().to_bytes());

    assert!(!is_reclaimable(&raw, &HashSet::new()), "not banned yet");
    let banned: HashSet<String> = std::iter::once(hex_key).collect();
    assert!(is_reclaimable(&raw, &banned), "ban frees the slot");
}

/// The line that must not move: absence is not departure. A member
/// who has been offline for years still owns their slot, or a closed
/// laptop becomes a membership event.
#[test]
fn a_long_offline_member_keeps_their_slot() {
    let signing = key(11);
    // Heartbeat at the epoch — as stale as a row can be.
    let raw = signed_row(&signing, 0);
    assert!(!is_reclaimable(&raw, &HashSet::new()));
}

/// A member who signed their own departure releases the slot. This
/// is the voluntary half; `banned_member_frees_the_slot` is the
/// involuntary one.
#[test]
fn a_signed_departure_frees_the_slot() {
    let signing = key(17);
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(signing.verifying_key().to_bytes()),
        status: "offline".into(),
        departed: true,
        last_heartbeat: 1_000,
        ..Default::default()
    };
    let sig = derive::sign_with_pseudonym(&signing, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let raw = serde_json::to_vec(&presence).expect("serialize");
    assert!(is_reclaimable(&raw, &HashSet::new()));
}

/// The eviction primitive that must not exist: a departure flag set
/// by somebody who does not hold the pseudonym. It fails W26, so it
/// frees the slot for being *forged* rather than for claiming
/// departure — either way the forger gains nothing, and a real
/// member's correctly-signed row is untouched by it.
#[test]
fn a_forged_departure_is_rejected_on_its_signature() {
    let victim = key(19);
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(victim.verifying_key().to_bytes()),
        status: "offline".into(),
        departed: true,
        last_heartbeat: 1_000,
        ..Default::default()
    };
    // Signed by the attacker, claiming to be the victim.
    let sig = derive::sign_with_pseudonym(&key(23), &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let raw = serde_json::to_vec(&presence).expect("serialize");
    assert!(is_reclaimable(&raw, &HashSet::new()));

    // And the victim's own live row still holds the slot, so the
    // forgery cannot displace them by racing it.
    let live = signed_row(&victim, 1_000);
    assert!(!is_reclaimable(&live, &HashSet::new()));
}

/// And an explicitly "offline" status is still a member.
#[test]
fn offline_status_keeps_their_slot() {
    let signing = key(13);
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(signing.verifying_key().to_bytes()),
        status: "offline".into(),
        last_heartbeat: 1_000,
        ..Default::default()
    };
    let sig = derive::sign_with_pseudonym(&signing, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let raw = serde_json::to_vec(&presence).expect("serialize");
    assert!(!is_reclaimable(&raw, &HashSet::new()));
}
