use rekindle_governance::state::{GovernanceState, InviteState};
use rekindle_types::id::PseudonymKey;

use super::warmable_record_keys;
use crate::state::community::CommunityState;

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
    let mut community = CommunityState::new("gov-id");
    community.name = "T".to_string();
    community.my_pseudonym_key = Some(hex::encode(my_pk.0));
    community.member_registry_key = Some("reg-key".to_string());
    community.anchor = Some("gov-key".to_string());
    community.governance_state = gov;
    community.open_community_records.governance_overflow_keys = overflow_keys;
    community
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
