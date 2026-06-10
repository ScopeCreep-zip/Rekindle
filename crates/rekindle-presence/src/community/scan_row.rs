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
    /// row has a fresh heartbeat and non-"offline" status (liveness
    /// only — route_blob may be empty); `None` for rows that count as
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
/// the architecture §3 gossip-overlay membership rules. Liveness is
/// decoupled from reachability — an online member may have an empty
/// `route_blob` (route not yet allocated); reachability consumers read
/// the blob off the snapshot and gate on it themselves:
/// - `status == "offline"` → not online (but still discovered for
///   member registry + role merging).
/// - `last_heartbeat <= stale_threshold` → stale; not online.
/// - otherwise → online (route_blob carried through, may be empty),
///   with `last_seen = now_secs`.
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
    // Liveness (am I here, now) ≠ reachability (can a route reach me).
    // "Offline" is a function of status + heartbeat freshness ONLY. The
    // route blob is allocated asynchronously and is frequently empty at
    // first write; gating liveness on it made freshly-joined, actively-
    // heartbeating members invisible to every peer until their route
    // landed (seconds-to-never). Route travels in
    // `OnlineMemberSnapshot.route_blob` (may be empty) for the separate
    // reachability consumers (DM / MEK delivery), which check it there.
    let is_offline = presence.status == "offline" || presence.last_heartbeat <= stale_cutoff;
    let online_member = if is_offline {
        None
    } else {
        Some(OnlineMemberSnapshot {
            route_blob: presence.route_blob.clone(),
            status: presence.status.clone(),
            last_seen: now_secs,
            // `location` lives in the MEK-encrypted SessionExtras; the
            // orchestrator decrypts + fills it after this pure classify.
            location: presence.session.location.clone(),
            last_active: presence.session.last_active,
        })
    };

    ClassifiedRow::Accepted(Box::new(AcceptedRow {
        pseudonym_hex,
        presence,
        online_member,
    }))
}

/// Gate a single registry-subkey payload down to a usable route blob
/// for one EXPECTED peer. Used by the gossip mesh's stale-route
/// re-resolve: the caller reads the peer's raw SMPL slot fresh from
/// the DHT and must not trust the bytes until they pass the same W26
/// signature + ban + liveness classification as the presence scan,
/// PLUS a pseudonym match — slot indices come from local SQLite and a
/// shifted or re-claimed slot must never hand back another member's
/// route.
#[must_use]
pub fn route_for_peer<S: BuildHasher>(
    raw_bytes: &[u8],
    expected_pseudonym_hex: &str,
    banned_pseudonyms: &HashSet<String, S>,
    stale_heartbeat_threshold_secs: u64,
    now_secs: u64,
) -> Option<Vec<u8>> {
    let classified = parse_and_classify_row(
        raw_bytes,
        banned_pseudonyms,
        stale_heartbeat_threshold_secs,
        now_secs,
    );
    let ClassifiedRow::Accepted(row) = classified else {
        return None;
    };
    if row.pseudonym_hex != expected_pseudonym_hex {
        return None;
    }
    let online = row.online_member?;
    if online.route_blob.is_empty() {
        return None;
    }
    Some(online.route_blob)
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

    #[test]
    fn route_for_peer_returns_blob_on_full_match() {
        let presence = signed_presence_for(&[7u8; 32], "c1", "online");
        let expected = hex::encode(presence.pseudonym_key.0);
        let bytes = serde_json::to_vec(&presence).unwrap();
        let banned = HashSet::new();
        let blob = route_for_peer(&bytes, &expected, &banned, 60, 1000);
        assert_eq!(blob, Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn route_for_peer_rejects_pseudonym_mismatch() {
        // The slot index came from local SQLite — if the slot was
        // re-claimed by a different member (or the index is shifted),
        // the row verifies but belongs to someone else. Must be None,
        // never another member's route.
        let presence = signed_presence_for(&[8u8; 32], "c1", "online");
        let bytes = serde_json::to_vec(&presence).unwrap();
        let banned = HashSet::new();
        let other = hex::encode([9u8; 32]);
        assert_eq!(route_for_peer(&bytes, &other, &banned, 60, 1000), None);
    }

    #[test]
    fn route_for_peer_rejects_offline_stale_banned_and_empty_blob() {
        let banned = HashSet::new();

        // Offline status.
        let offline = signed_presence_for(&[10u8; 32], "c1", "offline");
        let key = hex::encode(offline.pseudonym_key.0);
        let bytes = serde_json::to_vec(&offline).unwrap();
        assert_eq!(route_for_peer(&bytes, &key, &banned, 60, 1000), None);

        // Stale heartbeat (last_heartbeat=1000, cutoff=now-60=1940).
        let stale = signed_presence_for(&[11u8; 32], "c1", "online");
        let key = hex::encode(stale.pseudonym_key.0);
        let bytes = serde_json::to_vec(&stale).unwrap();
        assert_eq!(route_for_peer(&bytes, &key, &banned, 60, 2000), None);

        // Banned author.
        let banned_row = signed_presence_for(&[12u8; 32], "c1", "online");
        let key = hex::encode(banned_row.pseudonym_key.0);
        let bytes = serde_json::to_vec(&banned_row).unwrap();
        let mut ban_set = HashSet::new();
        ban_set.insert(key.clone());
        assert_eq!(route_for_peer(&bytes, &key, &ban_set, 60, 1000), None);

        // Empty route blob — online but unreachable.
        let mut routeless = signed_presence_for(&[13u8; 32], "c1", "online");
        routeless.route_blob.clear();
        let signing_key = derive_community_pseudonym(&[13u8; 32], "c1");
        let sig = sign_with_pseudonym(&signing_key, &routeless.signing_bytes());
        routeless.signature = sig.to_vec();
        let key = hex::encode(routeless.pseudonym_key.0);
        let bytes = serde_json::to_vec(&routeless).unwrap();
        assert_eq!(route_for_peer(&bytes, &key, &banned, 60, 1000), None);
    }

    #[test]
    fn route_for_peer_rejects_unverifiable_rows() {
        let banned = HashSet::new();
        let key = hex::encode([14u8; 32]);
        assert_eq!(route_for_peer(&[], &key, &banned, 60, 1000), None);
        let mut forged = signed_presence_for(&[14u8; 32], "c1", "online");
        forged.signature = vec![0u8; 64];
        let bytes = serde_json::to_vec(&forged).unwrap();
        assert_eq!(route_for_peer(&bytes, &key, &banned, 60, 1000), None);
    }
}
