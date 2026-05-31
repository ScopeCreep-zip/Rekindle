//! Pure per-row classification for the Plate Gate registry scan.
//!
//! Owns the W26 signature verification + ban filter + stale-heartbeat
//! / empty-route-blob classification. The adapter exposes raw bytes
//! per `(segment, subkey)` via `CommunityPresenceDeps::scan_segment_raw`;
//! the orchestrator calls [`parse_and_classify_row`] for each raw
//! payload to compute the discovered + online splits.

use std::collections::HashSet;
use std::hash::BuildHasher;

use rekindle_types::presence::MemberPresence;

use crate::deps::OnlineMemberSnapshot;

/// SMPL LOCAL subkeys per segment record (architecture §15.5).
/// Adapters pump 0..SUBKEYS_PER_SEGMENT through `get_dht_value`
/// when implementing `scan_segment_raw`.
pub const SUBKEYS_PER_SEGMENT: u32 = 255;

/// Outcome of one row's classification — either accepted (with the
/// MemberPresence body, hex-pseudonym, and whether it should be
/// treated as online right now) or rejected for one of the
/// documented reasons.
///
/// The `Accepted` variant carries a `MemberPresence` (~300 bytes
/// worst case with the full profile + W26 signature); the reject
/// variants are zero-sized markers. Boxed to keep the enum compact
/// and match the clippy `large_enum_variant` lint default.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassifiedRow {
    /// Row passed every check. `online_member` is `Some` when the
    /// row has a fresh heartbeat, non-empty route blob, and
    /// non-"offline" status; `None` for rows that count as
    /// "discovered" but don't enter the gossip overlay this tick.
    Accepted(Box<AcceptedRow>),
    /// Row's `MemberPresence` JSON couldn't be deserialised.
    MalformedJson,
    /// Row's signature wasn't a 64-byte buffer.
    InvalidSignatureLength,
    /// Row's signature didn't verify against its `pseudonym_key`
    /// (architecture §26 W26 — defends against forged presence
    /// writes by other slot-keypair holders).
    SignatureRejected,
    /// Row's author is in the community's ban list.
    Banned,
    /// Row's JSON payload was zero-length.
    EmptyPayload,
}

/// Payload for [`ClassifiedRow::Accepted`] — boxed inside the
/// enum so the discriminant stays small.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedRow {
    pub pseudonym_hex: String,
    pub presence: MemberPresence,
    pub online_member: Option<OnlineMemberSnapshot>,
}

/// Parse + verify + classify a single registry-subkey payload.
///
/// `raw_bytes` is the bytes returned by `get_dht_value`. Empty
/// payloads (subkey unused or fetch returned nothing) yield
/// [`ClassifiedRow::EmptyPayload`].
///
/// The `online_member` slot of [`ClassifiedRow::Accepted`] applies
/// the architecture §3 gossip-overlay membership rules:
/// - `status == "offline"` → not online (but still discovered for
///   member registry + role merging).
/// - `last_heartbeat <= stale_threshold` → stale; not online.
/// - empty `route_blob` → not reachable; not online.
/// - otherwise → online, with `last_seen = now_secs`.
#[must_use]
pub fn parse_and_classify_row<S: BuildHasher>(
    raw_bytes: &[u8],
    banned_pseudonyms: &HashSet<String, S>,
    stale_heartbeat_threshold_secs: u64,
    now_secs: u64,
) -> ClassifiedRow {
    if raw_bytes.is_empty() {
        return ClassifiedRow::EmptyPayload;
    }
    let Ok(presence) = serde_json::from_slice::<MemberPresence>(raw_bytes) else {
        return ClassifiedRow::MalformedJson;
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(presence.signature.as_slice()) else {
        return ClassifiedRow::InvalidSignatureLength;
    };
    if rekindle_secrets::derive::verify_pseudonym_signature(
        &presence.pseudonym_key.0,
        &presence.signing_bytes(),
        &sig_arr,
    )
    .is_err()
    {
        return ClassifiedRow::SignatureRejected;
    }
    let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
    if banned_pseudonyms.contains(&pseudonym_hex) {
        return ClassifiedRow::Banned;
    }

    let stale_cutoff = now_secs.saturating_sub(stale_heartbeat_threshold_secs);
    // Collapse the three "treat as offline" reasons (status, stale
    // heartbeat, empty route blob) into a single check.
    let is_offline = presence.status == "offline"
        || presence.last_heartbeat <= stale_cutoff
        || presence.route_blob.is_empty();
    let online_member = if is_offline {
        None
    } else {
        Some(OnlineMemberSnapshot {
            route_blob: presence.route_blob.clone(),
            status: presence.status.clone(),
            last_seen: now_secs,
        })
    };

    ClassifiedRow::Accepted(Box::new(AcceptedRow {
        pseudonym_hex,
        presence,
        online_member,
    }))
}

#[cfg(test)]
mod tests {
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
    fn empty_route_blob_yields_no_online_slot() {
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
        let online_member = row.online_member;
        assert!(online_member.is_none());
    }
}
