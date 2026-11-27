use super::*;
use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};
use rekindle_types::id::PseudonymKey;

fn signed_presence_for(seed: &[u8; 32], community: &str, status: &str) -> MemberPresence {
    let signing_key = derive_community_pseudonym(seed, community);
    // Reach the verifying-key bytes via the returned
    // `SigningKey` without naming the `ed25519_dalek` types —
    // keeps this test module clear of the ed25519-dalek
    // boundary that xtask enforces on the production crate
    // (only `rekindle-secrets` may import the underlying
    // crypto types).
    let pseudonym_bytes = signing_key.verifying_key().to_bytes();
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(pseudonym_bytes),
        display_name: Some("alice".to_string()),
        status: status.to_string(),
        route_blob: vec![1, 2, 3, 4],
        last_heartbeat: 1000,
        ..Default::default()
    };
    let sig = sign_with_pseudonym(&signing_key, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    presence
}

#[test]
fn empty_payload_yields_empty_classification() {
    let banned = HashSet::new();
    let result = parse_and_classify_row(&[], &banned, 60, 1000);
    assert!(matches!(result, ClassifiedRow::EmptyPayload));
}

#[test]
fn malformed_json_is_rejected() {
    let banned = HashSet::new();
    let result = parse_and_classify_row(b"\xff\xff\xff not json", &banned, 60, 1000);
    assert!(matches!(result, ClassifiedRow::MalformedJson));
}

#[test]
fn missing_signature_is_rejected() {
    let presence = MemberPresence {
        pseudonym_key: PseudonymKey([1u8; 32]),
        status: "online".into(),
        route_blob: vec![1, 2, 3],
        last_heartbeat: 1000,
        signature: Vec::new(),
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 1000);
    assert!(matches!(result, ClassifiedRow::InvalidSignatureLength));
}

#[test]
fn tampered_signature_is_rejected() {
    let mut presence = signed_presence_for(&[1u8; 32], "c1", "online");
    presence.signature = vec![0u8; 64]; // zero sig — won't verify
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 2000);
    assert!(matches!(result, ClassifiedRow::SignatureRejected));
}

#[test]
fn banned_member_is_rejected_even_with_valid_signature() {
    let presence = signed_presence_for(&[2u8; 32], "c1", "online");
    let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
    let bytes = serde_json::to_vec(&presence).unwrap();
    let mut banned = HashSet::new();
    banned.insert(pseudonym_hex);
    let result = parse_and_classify_row(&bytes, &banned, 60, 1000);
    assert!(matches!(result, ClassifiedRow::Banned));
}

#[test]
fn fresh_online_row_is_accepted_with_online_member() {
    let presence = signed_presence_for(&[3u8; 32], "c1", "online");
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 1000);
    let ClassifiedRow::Accepted(row) = result else {
        panic!("expected Accepted");
    };
    let pseudonym_hex = row.pseudonym_hex;
    let online_member = row.online_member;
    assert!(!pseudonym_hex.is_empty());
    let online = online_member.expect("online slot");
    assert_eq!(online.route_blob, vec![1, 2, 3, 4]);
    assert_eq!(online.status, "online");
    assert_eq!(online.last_seen, 1000);
}

#[test]
fn offline_status_yields_no_online_slot() {
    let presence = signed_presence_for(&[4u8; 32], "c1", "offline");
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 1000);
    let ClassifiedRow::Accepted(row) = result else {
        panic!("expected Accepted");
    };
    let online_member = row.online_member;
    assert!(online_member.is_none());
}

#[test]
fn stale_heartbeat_yields_no_online_slot() {
    // last_heartbeat = 1000, stale_threshold = 60, now = 2000 →
    // cutoff = 1940 → 1000 ≤ 1940 → stale.
    let presence = signed_presence_for(&[5u8; 32], "c1", "online");
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 2000);
    let ClassifiedRow::Accepted(row) = result else {
        panic!("expected Accepted");
    };
    let online_member = row.online_member;
    assert!(online_member.is_none());
}

#[test]
fn empty_route_blob_still_online() {
    // Liveness ≠ reachability: a fresh, non-offline heartbeat with
    // no route allocated yet is STILL online (the live "can't see
    // each other" bug was this row being classified offline). The
    // empty route is carried through on the snapshot for separate
    // reachability consumers.
    let mut presence = signed_presence_for(&[6u8; 32], "c1", "online");
    // Strip route + re-sign so the row still verifies.
    presence.route_blob.clear();
    let signing_key = derive_community_pseudonym(&[6u8; 32], "c1");
    let sig = sign_with_pseudonym(&signing_key, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let bytes = serde_json::to_vec(&presence).unwrap();
    let banned = HashSet::new();
    let result = parse_and_classify_row(&bytes, &banned, 60, 1000);
    let ClassifiedRow::Accepted(row) = result else {
        panic!("expected Accepted");
    };
    let online = row.online_member.expect("routeless member is still online");
    assert!(online.route_blob.is_empty());
    assert_eq!(online.status, "online");
    assert_eq!(online.last_seen, 1000);
}
