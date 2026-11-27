use super::*;
use crate::state::community::CommunityRecords;
use rekindle_governance::state::{GovernanceState, InviteState};
use rekindle_types::id::PseudonymKey;
use std::collections::{HashMap, HashSet, VecDeque};

fn invite(creator: PseudonymKey, secrets_key: &str, expires_at: Option<u64>) -> InviteState {
    InviteState {
        code_hash: String::new(),
        max_uses: 1,
        expires_at,
        secrets_record_key: secrets_key.to_string(),
        created: rekindle_governance::op::OpStamp {
            lamport: 0,
            hash: rekindle_types::governance::OpHash([0; 32]),
        },
        creator_pseudonym: creator,
        uses: 0,
    }
}

/// Minimal `CommunityState` for inventory assertions — every field set to an
/// empty/None default except the record-key fields the inventory reads.
fn community(
    my_pk: &PseudonymKey,
    gov: Option<GovernanceState>,
    overflow_keys: Vec<String>,
) -> CommunityState {
    CommunityState {
        genesis_cert: None,
        key_pending_since_ms: None,
        published_key_status: None,
        held_threads: std::collections::HashSet::new(),
        id: "gov-id".to_string(),
        name: "T".to_string(),
        description: None,
        channels: Vec::new(),
        categories: Vec::new(),
        my_role_ids: Vec::new(),
        roles: Vec::new(),
        my_pseudonym_key: Some(hex::encode(my_pk.0)),
        mek_generation: 0,
        member_registry_key: Some("reg-key".to_string()),
        my_subkey_index: None,
        my_segment_index: None,
        segments: Vec::new(),
        watch_ring: std::collections::BTreeMap::new(),
        sealed_pages: rekindle_node::host::community::sealed_pages::SealedPages::default(),
        removal_watch: std::collections::BTreeMap::new(),
        key_drop: None,
        own_record_watch: None,
        own_key_retired_shown: false,
        key_drop_writer: std::sync::Arc::default(),
        beat_view: rekindle_presence::community::BeatView::default(),
        on_screen: rekindle_presence::community::OnScreen::default(),
        member_routes: rekindle_presence::community::MemberRouteCache::default(),
        head_hints: rekindle_node::host::community::state::HeadHints::default(),
        outbox_streams: std::sync::Arc::default(),
        outbox_unmapped: std::sync::Arc::default(),
        snapshot_requests: rekindle_transport::gossip::RateLimiter::default(),
        anchor: Some("gov-key".to_string()),
        my_member_record: None,
        governance_state: gov,
        own_removal: None,
        removal_gate: std::sync::Arc::default(),
        message_clock: 0,
        gossip: None,
        slot_keypair: None,
        registry_owner_keypair: None,
        slot_seed: None,
        known_members: HashSet::new(),
        watched_records: HashSet::new(),
        record_sequences: HashMap::new(),
        channel_last_send_at: HashMap::new(),
        peer_reliability: HashMap::new(),
        tasks: None,
        leases: rekindle_records::lease::CommunityLeases::default(),
        loops_started: false,
        departing: false,
        departure_running: false,
        open_community_records: CommunityRecords {
            governance_overflow_keys: overflow_keys,
            ..CommunityRecords::default()
        },
        my_event_rsvps: HashMap::new(),
        event_rsvps_by_event: HashMap::new(),
        onboarding_complete: false,
        my_bio: None,
        my_pronouns: None,
        my_theme_color: None,
        my_badges: Vec::new(),
        my_avatar_ref: None,
        my_banner_ref: None,
        icon_hash: None,
        banner_hash: None,
        member_profiles: HashMap::new(),
        recent_member_joins: VecDeque::new(),
        my_session_location: None,
        presence_policy: rekindle_types::presence::PresenceSharingPolicy::default(),
        members: std::collections::HashMap::new(),
        synced_channels: std::collections::BTreeSet::new(),
    }
}

#[test]
fn includes_core_overflow_and_my_invites_excludes_foreign_invites() {
    let me = PseudonymKey([7u8; 32]);
    let other = PseudonymKey([9u8; 32]);
    let mut gov = GovernanceState::default();
    gov.invites
        .insert([1u8; 16], invite(me.clone(), "my-secrets", None));
    gov.invites
        .insert([2u8; 16], invite(other, "other-secrets", None));
    // An expired invite of mine must NOT be warmed.
    gov.invites
        .insert([3u8; 16], invite(me.clone(), "my-expired", Some(1)));

    let cs = community(&me, Some(gov), vec!["ovl-key".to_string()]);
    let keys = warmable_record_keys(&cs);

    assert!(keys.contains(&"gov-key".to_string()), "anchor key");
    assert!(keys.contains(&"reg-key".to_string()), "registry key");
    assert!(keys.contains(&"ovl-key".to_string()), "overflow key");
    assert!(keys.contains(&"my-secrets".to_string()), "my active invite");
    assert!(
        !keys.contains(&"other-secrets".to_string()),
        "another author's invite must be excluded"
    );
    assert!(
        !keys.contains(&"my-expired".to_string()),
        "my expired invite must be excluded"
    );
}

#[test]
fn falls_back_to_community_id_when_no_anchor() {
    let me = PseudonymKey([7u8; 32]);
    let mut cs = community(&me, None, Vec::new());
    cs.anchor = None;
    let keys = warmable_record_keys(&cs);
    assert!(
        keys.contains(&"gov-id".to_string()),
        "must fall back to community id as the governance record key"
    );
}

#[test]
fn result_is_sorted_and_deduped() {
    let me = PseudonymKey([7u8; 32]);
    // An overflow key duplicated to prove dedup.
    let cs = community(
        &me,
        Some(GovernanceState::default()),
        vec!["ovl-a".to_string(), "ovl-a".to_string()],
    );
    let keys = warmable_record_keys(&cs);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "keys must be sorted");
    let mut deduped = keys.clone();
    deduped.dedup();
    assert_eq!(keys, deduped, "keys must be deduped");
}
