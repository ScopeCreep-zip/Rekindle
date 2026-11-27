use super::*;

#[test]
fn default_community_name_short_anchor() {
    assert_eq!(default_community_name("abc"), "Community abc");
}

#[test]
fn default_community_name_truncates_to_eight() {
    assert_eq!(
        default_community_name("abcdefghijklmnop"),
        "Community abcdefgh"
    );
}

#[test]
fn derive_join_identity_is_deterministic() {
    let secret = [1u8; 32];
    let gov_key = "test-community-key";
    let a = derive_join_identity(&secret, gov_key);
    let b = derive_join_identity(&secret, gov_key);
    assert_eq!(a.pseudo_hex, b.pseudo_hex);
    assert_eq!(a.pseudo.0, b.pseudo.0);
}

#[test]
fn derive_join_identity_differs_by_community() {
    let secret = [1u8; 32];
    let a = derive_join_identity(&secret, "community-a");
    let b = derive_join_identity(&secret, "community-b");
    assert_ne!(a.pseudo_hex, b.pseudo_hex);
}

#[test]
fn merge_presence_offline_only_records_known() {
    let mut presence = InitialPresence::default();
    merge_presence_entry(&mut presence, "abc", "offline", &[1, 2, 3], 100);
    assert!(presence.known_members.contains("abc"));
    assert!(presence.peers.is_empty());
    assert!(presence.online.is_empty());
}

#[test]
fn merge_presence_empty_route_is_online_but_unreachable() {
    // Liveness ≠ reachability: a fresh, non-offline member with no route
    // allocated yet is STILL online (this was the "can't see each other at
    // join" bug). The empty route keeps it out of `peers` (unreachable
    // until a route lands) but it MUST appear in `online`.
    let mut presence = InitialPresence::default();
    merge_presence_entry(&mut presence, "abc", "online", &[], 100);
    assert!(presence.known_members.contains("abc"));
    assert!(presence.peers.is_empty());
    assert_eq!(presence.online.len(), 1);
    assert_eq!(presence.online["abc"].last_seen, 100);
    assert!(presence.online["abc"].route_blob.is_empty());
}

#[test]
fn merge_presence_online_with_route_populates_all() {
    let mut presence = InitialPresence::default();
    merge_presence_entry(&mut presence, "abc", "online", &[7, 8, 9], 42);
    assert!(presence.known_members.contains("abc"));
    assert_eq!(presence.peers.len(), 1);
    assert_eq!(presence.online.len(), 1);
    assert_eq!(presence.peers["abc"].route_blob, vec![7, 8, 9]);
    assert_eq!(presence.online["abc"].last_seen, 42);
}

#[test]
fn find_invite_returns_inviter_pseudonym() {
    let author = PseudonymKey([7u8; 32]);
    let entries = vec![(
        author.clone(),
        vec![GovernanceEntry::InviteCreated {
            invite_id: [1u8; 16],
            code_hash: "abc".into(),
            max_uses: 0,
            expires_at: None,
            secrets_record_key: "VLD0:record-key".into(),
        }],
    )];
    let (record_key, inviter) =
        find_invite_in_entries(&entries, "abc", rekindle_utils::timestamp_secs()).expect("found");
    assert_eq!(record_key, "VLD0:record-key");
    assert_eq!(inviter.0, author.0);
}

#[test]
fn find_invite_rejects_revoked() {
    let author = PseudonymKey([7u8; 32]);
    let entries = vec![(
        author.clone(),
        vec![
            GovernanceEntry::InviteCreated {
                invite_id: [1u8; 16],
                code_hash: "abc".into(),
                max_uses: 0,
                expires_at: None,
                secrets_record_key: "x".into(),
            },
            GovernanceEntry::InviteRevoked {
                invite_id: [1u8; 16],
            },
        ],
    )];
    let err = find_invite_in_entries(&entries, "abc", rekindle_utils::timestamp_secs())
        .expect_err("revoked");
    match err {
        GovernanceRuntimeError::Adapter(msg) => assert!(msg.contains("revoked")),
        other => panic!("expected Adapter(revoked), got {other:?}"),
    }
}

#[test]
fn find_invite_no_match_errors() {
    let entries = vec![];
    let err = find_invite_in_entries(&entries, "nope", rekindle_utils::timestamp_secs())
        .expect_err("no match");
    match err {
        GovernanceRuntimeError::Adapter(msg) => assert!(msg.contains("no matching invite")),
        other => panic!("expected Adapter(no matching invite), got {other:?}"),
    }
}

#[test]
fn inspect_invite_active_carries_inviter_and_secrets() {
    let author = PseudonymKey([9u8; 32]);
    let entries = vec![(
        author.clone(),
        vec![GovernanceEntry::InviteCreated {
            invite_id: [2u8; 16],
            code_hash: "live".into(),
            max_uses: 0,
            expires_at: None,
            secrets_record_key: "VLD0:secrets".into(),
        }],
    )];
    match inspect_invite_in_entries(&entries, "live", rekindle_utils::timestamp_secs()) {
        InviteGovStatus::Active {
            inviter,
            secrets_record_key,
        } => {
            assert_eq!(inviter.0, author.0);
            assert_eq!(secrets_record_key, "VLD0:secrets");
        }
        _ => panic!("expected Active"),
    }
}

#[test]
fn inspect_invite_expired_when_past_expiry() {
    let author = PseudonymKey([3u8; 32]);
    let entries = vec![(
        author,
        vec![GovernanceEntry::InviteCreated {
            invite_id: [4u8; 16],
            code_hash: "old".into(),
            max_uses: 0,
            // Unix epoch + 1s — unconditionally in the past.
            expires_at: Some(1),
            secrets_record_key: "x".into(),
        }],
    )];
    assert!(matches!(
        inspect_invite_in_entries(&entries, "old", rekindle_utils::timestamp_secs()),
        InviteGovStatus::Expired
    ));
}
