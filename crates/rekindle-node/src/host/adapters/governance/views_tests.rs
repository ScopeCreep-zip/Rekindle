//! A community's window view (plan E3.1 D1b).

use super::{community_view, moderation_of, role_view};
use crate::host::community::{CommunityState, RoleDefinition};

fn owner_role() -> RoleDefinition {
    RoleDefinition {
        id: rekindle_types::id::RoleId::EVERYONE,
        name: "Owner".into(),
        color: 0x00ff_aa00,
        permissions: u64::MAX >> 1,
        position: 100,
        hoist: true,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    }
}

#[test]
fn role_permissions_ride_as_decimal_text() {
    let view = role_view(&owner_role(), None);
    assert_eq!(view.permissions, (u64::MAX >> 1).to_string());
    let json = serde_json::to_value(&view).unwrap();
    assert!(json["permissions"].is_string(), "{json}");
    assert!(json.get("selfAssignable").is_some(), "camelCase: {json}");
}

#[test]
fn the_view_names_our_role_and_carries_our_standing() {
    let mut state = CommunityState::new("gk");
    state.name = "Raiders".into();
    state.roles = vec![owner_role()];
    state.my_pseudonym_key = Some("ab".repeat(32));
    // Our role ids are the projection's (review RV7 M1).
    let mut gov = rekindle_governance::state::GovernanceState::default();
    gov.role_assignments.insert(
        rekindle_types::id::PseudonymKey([0xab; 32]),
        std::collections::HashSet::from([rekindle_types::id::RoleId::EVERYONE]),
    );
    crate::host::adapters::governance::projection::project_my_roles(&mut state, &gov);
    state.mek_generation = 3;
    let view = community_view(&state, own_card_view(&state, None), 0, None);
    assert_eq!(view.name, "Raiders");
    assert_eq!(view.my_role.as_deref(), Some("Owner"));
    assert_eq!(view.my_role_ids, vec!["00".repeat(16)]);
    assert_eq!(view.mek_generation, 3);
    assert_eq!(view.roles.len(), 1);
    let json = serde_json::to_value(&view).unwrap();
    assert!(json.get("myPseudonymKey").is_some(), "{json}");
    assert!(
        json.get("myBio").is_none(),
        "unset profile fields are omitted"
    );
}

/// A creator (1), an admin (5, position 5, ADMINISTRATOR) and a moderator
/// (6, position 2, BAN only).
fn hierarchy() -> rekindle_governance::state::GovernanceState {
    use rekindle_governance::state::RoleState;
    use rekindle_types::id::{PseudonymKey, RoleId};
    use rekindle_types::permissions::{ADMINISTRATOR, BAN_MEMBERS};
    let role = |permissions, position| RoleState {
        name: "r".into(),
        permissions,
        position,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: rekindle_governance::fixture::stamp(1),
    };
    let mut gov = rekindle_governance::state::GovernanceState {
        creators: std::collections::BTreeSet::from([PseudonymKey([1; 32])]),
        ..Default::default()
    };
    gov.roles.insert(RoleId([5; 16]), role(ADMINISTRATOR, 5));
    gov.roles.insert(RoleId([2; 16]), role(BAN_MEMBERS, 2));
    for (member, role) in [(5u8, 5u8), (6, 2)] {
        gov.role_assignments.insert(
            PseudonymKey([member; 32]),
            std::collections::HashSet::from([RoleId([role; 16])]),
        );
    }
    gov
}

#[test]
fn each_member_rows_flags_are_the_validators_verdicts() {
    use rekindle_governance::validate::validate_write;
    use rekindle_types::governance::GovernanceEntry;
    use rekindle_types::id::PseudonymKey;
    let gov = hierarchy();
    let pk = |b: u8| PseudonymKey([b; 32]);
    for me in [1u8, 5, 6] {
        for target in [1u8, 5, 6] {
            let (flags, outranks) = moderation_of(&gov, Some(&pk(me)), &pk(target));
            if me == target {
                assert_eq!(
                    (flags, outranks),
                    Default::default(),
                    "nothing to ourselves"
                );
                continue;
            }
            let kick = GovernanceEntry::MemberRemoved {
                target: pk(target),
                reason: None,
                heads: Vec::new(),
                target_seq_frontier: 0,
            };
            let ban = GovernanceEntry::BanEntry {
                target: pk(target),
                reason: None,
                target_seq_frontier: 0,
            };
            let timeout = GovernanceEntry::TimeoutEntry {
                target: pk(target),
                duration_seconds: 60,
                reason: None,
                started_at: 0,
            };
            let at = format!("{me} on {target}");
            assert_eq!(
                flags.kick,
                validate_write(&pk(me), &kick, &gov),
                "kick {at}"
            );
            assert_eq!(flags.ban, validate_write(&pk(me), &ban, &gov), "ban {at}");
            assert_eq!(
                flags.timeout,
                validate_write(&pk(me), &timeout, &gov),
                "timeout {at}"
            );
        }
    }
    // The moderator may ban below its rank only: never the admin above it.
    let (flags, outranks) = moderation_of(&gov, Some(&pk(5)), &pk(6));
    assert!(flags.kick && flags.ban && flags.timeout && outranks);
    let (flags, outranks) = moderation_of(&gov, Some(&pk(6)), &pk(5));
    assert!(!flags.ban && !outranks);
    assert_eq!(moderation_of(&gov, None, &pk(6)), Default::default());
}

/// A member just admitted (or just rejoined) holds no role assignment yet:
/// its row's verdicts are still the validators', from the same merged
/// governance that lists it, so the member menu has its actions as soon as
/// the row is listed (plan MM1, M87): the creator and the admin may kick,
/// ban and time it out and outrank it.
#[test]
fn a_just_admitted_members_row_carries_its_verdicts() {
    use rekindle_types::id::PseudonymKey;
    let gov = hierarchy();
    let pk = |b: u8| PseudonymKey([b; 32]);
    let fresh = pk(9);
    assert!(!gov.role_assignments.contains_key(&fresh));
    for me in [1u8, 5] {
        let (flags, outranks) = moderation_of(&gov, Some(&pk(me)), &fresh);
        assert!(
            flags.kick && flags.ban && flags.timeout && outranks,
            "{me} on a just-admitted member"
        );
    }
}

/// Two roles whose first four bytes match (what a `u32` id kept) stay two
/// roles at every surface: a member holding one is never shown holding
/// the other (plan E3.2 S9, the 16-byte `RoleId` item).
#[test]
fn roles_sharing_their_low_four_bytes_never_collide_in_the_view() {
    use rekindle_governance::state::RoleState;
    use rekindle_types::id::{PseudonymKey, RoleId};
    let role = |name: &str, position| RoleState {
        name: name.into(),
        permissions: 0,
        position,
        color: 0,
        hoist: true,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: rekindle_governance::fixture::stamp(1),
    };
    let alpha = RoleId([1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let beta = RoleId([1, 2, 3, 4, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
    let mut gov = rekindle_governance::state::GovernanceState::default();
    gov.roles.insert(alpha, role("Alpha", 1));
    gov.roles.insert(beta, role("Beta", 2));
    gov.role_assignments.insert(
        PseudonymKey([0xab; 32]),
        std::collections::HashSet::from([beta]),
    );
    let mut state = CommunityState::new("gk");
    state.my_pseudonym_key = Some("ab".repeat(32));
    crate::host::adapters::governance::projection::project(&mut state, gov.clone());
    let json =
        serde_json::to_value(community_view(&state, own_card_view(&state, None), 0, None)).unwrap();
    let id_of = |name: &str| {
        json["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .map(|r| r["id"].clone())
            .unwrap()
    };
    let (alpha_id, beta_id) = (id_of("Alpha"), id_of("Beta"));
    assert_ne!(alpha_id, beta_id, "two roles, two ids: {json}");
    assert_eq!(json["myRoleIds"], serde_json::json!([beta_id]), "{json}");
    let members = crate::host::adapters::governance::projection::member_role_ids(&gov);
    assert_eq!(members[&"ab".repeat(32)], vec![beta]);
    assert_eq!(beta_id, serde_json::json!(beta.to_hex()));
    assert_eq!(alpha_id, serde_json::json!(alpha.to_hex()));
    assert_eq!(json["myRole"], "Beta", "{json}");
}

/// Lead decision (53): `weAreCreator` follows our seat, so a co-creator's
/// view loses it on a real merged `MemberLeft`.
#[test]
fn we_are_creator_is_false_after_we_leave() {
    use rekindle_governance::fixture::{at, cert, merge_entries_under};
    use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry};
    use rekindle_types::id::PseudonymKey;
    let (primary, us) = (PseudonymKey([0x01; 32]), PseudonymKey([0xab; 32]));
    let cert = GenesisCert {
        additional_creators: vec![us.clone()],
        ..cert(&primary, AdmissionMode::default())
    };
    let joined = GovernanceEntry::MemberJoined {
        member_record: "VLD0:record-ab".into(),
        member_record_owner: "owner-ab".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: None,
    };
    let left = GovernanceEntry::MemberLeft { heads: Vec::new() };
    let mut state = CommunityState::new("gk");
    state.my_pseudonym_key = Some("ab".repeat(32));
    state.governance_state = Some(merge_entries_under(
        &cert,
        &[(us.clone(), vec![at(5, joined.clone())])],
    ));
    assert!(community_view(&state, own_card_view(&state, None), 0, None).we_are_creator);
    state.governance_state = Some(merge_entries_under(
        &cert,
        &[(us, vec![at(5, joined), at(6, left)])],
    ));
    assert!(!community_view(&state, own_card_view(&state, None), 0, None).we_are_creator);
}
