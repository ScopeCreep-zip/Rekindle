//! A community's window view (plan E3.1 D1b).

use super::{community_view, moderation_of, own_card_view, role_view};
use crate::host::adapters::governance::capability_words::{JOIN_UNFINISHED, YOURSELF};
use crate::host::community::{CommunityState, RoleDefinition};
use rekindle_types::dto::community::MemberModeration;

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

fn assert_channel_overwrite_words(bit: u64, key: &str, reason: &str) {
    use crate::host::adapters::governance::projection::project;
    use rekindle_governance::{fixture::stamp, merge::apply_entry};
    use rekindle_types::governance::GovernanceEntry;
    use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
    use rekindle_types::permissions::DEFAULT_EVERYONE;

    let creator = PseudonymKey([1; 32]);
    let member = PseudonymKey([6; 32]);
    let member_role = RoleId([2; 16]);
    let channel = ChannelId([9; 16]);
    let mut gov = hierarchy();
    gov.roles.get_mut(&member_role).unwrap().permissions = DEFAULT_EVERYONE | bit;
    apply_entry(
        &creator,
        &GovernanceEntry::ChannelCreated {
            channel_id: channel,
            name: "general".into(),
            channel_type: "text".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
        },
        stamp(2),
        &mut gov,
    );

    let mut state = CommunityState::new("gk");
    state.my_pseudonym_key = Some(member.to_hex());
    let denied = |state: &CommunityState| {
        let view = community_view(state, own_card_view(state, None), 0, None);
        let json = serde_json::to_value(view).unwrap();
        json["channels"][0]["denied"].as_object().unwrap().clone()
    };
    project(&mut state, gov.clone());
    let mut expected = denied(&state);
    assert_eq!(expected.insert(key.to_owned(), reason.into()), None);

    apply_entry(
        &creator,
        &GovernanceEntry::PermissionOverwrite {
            channel_id: channel,
            target_type: "member".into(),
            target_id: member.to_hex(),
            allow: 0,
            deny: bit,
        },
        stamp(3),
        &mut gov,
    );
    project(&mut state, gov);
    assert_eq!(denied(&state), expected);
}

#[test]
fn send_messages_overwrite_is_worded_in_the_channel_view() {
    assert_channel_overwrite_words(
        rekindle_types::permissions::SEND_MESSAGES,
        "sendMessages",
        "Send Messages is turned off for you in this channel.",
    );
}

#[test]
fn bypass_slowmode_overwrite_is_worded_in_the_channel_view() {
    assert_channel_overwrite_words(
        rekindle_types::permissions::BYPASS_SLOWMODE,
        "bypassSlowmode",
        "Bypass Slowmode is turned off for you in this channel.",
    );
}

#[test]
fn request_to_speak_overwrite_is_worded_in_the_channel_view() {
    assert_channel_overwrite_words(
        rekindle_types::permissions::REQUEST_TO_SPEAK,
        "requestToSpeak",
        "Request to Speak is turned off for you in this channel.",
    );
}

#[test]
fn mention_everyone_overwrite_is_worded_in_the_channel_view() {
    assert_channel_overwrite_words(
        rekindle_types::permissions::MENTION_EVERYONE,
        "mentionEveryone",
        "Mention Everyone is turned off for you in this channel.",
    );
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
                    (MemberModeration::refused(YOURSELF), false),
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
                flags.kick.is_none(),
                validate_write(&pk(me), &kick, &gov),
                "kick {at}"
            );
            assert_eq!(
                flags.ban.is_none(),
                validate_write(&pk(me), &ban, &gov),
                "ban {at}"
            );
            assert_eq!(
                flags.timeout.is_none(),
                validate_write(&pk(me), &timeout, &gov),
                "timeout {at}"
            );
        }
    }
    // The moderator may ban below its rank only: never the admin above it.
    let (flags, outranks) = moderation_of(&gov, Some(&pk(5)), &pk(6));
    assert!(flags.kick.is_none() && flags.ban.is_none() && flags.timeout.is_none() && outranks);
    let (flags, outranks) = moderation_of(&gov, Some(&pk(6)), &pk(5));
    assert!(flags.ban.is_some() && !outranks);
    assert_eq!(
        moderation_of(&gov, None, &pk(6)),
        (MemberModeration::refused(JOIN_UNFINISHED), false)
    );
}

/// The role menu is refused with its real cause when the moderator holds
/// Manage Roles but no role can be given (plan E3.2 S9): none exist, or
/// every one sits at or above the moderator's highest role.
#[test]
fn manage_roles_names_why_no_role_can_be_given() {
    use rekindle_types::id::{PseudonymKey, RoleId};
    use rekindle_types::permissions::{MANAGE_ROLES, VIEW_CHANNELS};
    let pk = |b: u8| PseudonymKey([b; 32]);
    let creator_only = rekindle_governance::state::GovernanceState {
        creators: std::collections::BTreeSet::from([pk(1)]),
        ..Default::default()
    };
    let (flags, _) = moderation_of(&creator_only, Some(&pk(1)), &pk(9));
    assert_eq!(
        flags.manage_roles.as_deref(),
        Some("This community has no roles yet.")
    );
    // A manager whose only role is the lowest: it outranks a bare member
    // but no role ranks below its own.
    let mut gov = hierarchy();
    let mut manager = gov.roles[&RoleId([2; 16])].clone();
    manager.permissions = VIEW_CHANNELS | MANAGE_ROLES;
    manager.position = 1;
    gov.roles.insert(RoleId([1; 16]), manager);
    gov.role_assignments
        .insert(pk(7), std::collections::HashSet::from([RoleId([1; 16])]));
    let (flags, outranks) = moderation_of(&gov, Some(&pk(7)), &pk(9));
    assert!(outranks);
    assert_eq!(
        flags.manage_roles.as_deref(),
        Some("There are no roles you can assign.")
    );
    // The admin has roles below its own to give.
    let (flags, _) = moderation_of(&gov, Some(&pk(5)), &pk(9));
    assert_eq!(flags.manage_roles, None);
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
            flags.kick.is_none() && flags.ban.is_none() && flags.timeout.is_none() && outranks,
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

/// The unlock barrier's view rides on the window's re-read as `catchUp`
/// (plan E3.2 S7d-keys.8), camelCase like `GovernanceEvent::CatchUp`, and
/// is omitted once the barrier passed with nothing held.
#[test]
fn the_view_carries_the_unlock_barriers_held_writes_as_catch_up() {
    use rekindle_types::display_catch_up::{
        CatchUpView, PendingGovKind, PendingGovOp, PendingGovState, PendingGovTarget,
    };
    let state = CommunityState::new("gk");
    let mut view = community_view(&state, own_card_view(&state, None), 0, None);
    let idle = serde_json::to_value(&view).unwrap();
    assert!(idle.get("catchUp").is_none(), "{idle}");
    view.catch_up = Some(CatchUpView {
        catching_up: true,
        pending: vec![PendingGovOp {
            op_id: "op1".into(),
            kind: PendingGovKind::Kick,
            target: PendingGovTarget {
                member: Some("cd".repeat(32)),
                ..PendingGovTarget::default()
            },
            label: "Kick Bea".into(),
            state: PendingGovState::CatchingUp,
        }],
    });
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["catchUp"]["catchingUp"], true, "{json}");
    let held = &json["catchUp"]["pending"][0];
    assert_eq!(held["opId"], "op1", "{json}");
    assert_eq!(held["kind"], "kick", "{json}");
    assert_eq!(held["state"], "catchingUp", "{json}");
    assert_eq!(held["target"]["member"], "cd".repeat(32), "{json}");
}

/// Server mute and deafen carry the Host's reasons (plan E3.2 S9): the
/// missing permission, and a target at or above our highest role. No
/// voice-state reason: the Host learns other members' voice state in
/// E3.4 S8b.
#[test]
fn mute_and_deafen_name_the_permission_and_the_rank() {
    use rekindle_types::id::{PseudonymKey, RoleId};
    use rekindle_types::permissions::{CONNECT, DEAFEN_MEMBERS, MUTE_MEMBERS, VIEW_CHANNELS};
    let pk = |b: u8| PseudonymKey([b; 32]);
    let mut gov = hierarchy();
    let mut voice = gov.roles[&RoleId([2; 16])].clone();
    voice.permissions = VIEW_CHANNELS | CONNECT | MUTE_MEMBERS | DEAFEN_MEMBERS;
    voice.position = 3;
    gov.roles.insert(RoleId([3; 16]), voice);
    gov.role_assignments
        .insert(pk(7), std::collections::HashSet::from([RoleId([3; 16])]));
    let rank = Some("You can't moderate someone at or above your highest role.");
    let (flags, _) = moderation_of(&gov, Some(&pk(7)), &pk(5));
    assert_eq!(
        (flags.mute.as_deref(), flags.deafen.as_deref()),
        (rank, rank)
    );
    let (flags, _) = moderation_of(&gov, Some(&pk(7)), &pk(6));
    assert_eq!((flags.mute, flags.deafen), (None, None));
    // The ban-only moderator lacks both voice permissions.
    let (flags, _) = moderation_of(&gov, Some(&pk(6)), &pk(9));
    assert_eq!(
        (flags.mute.as_deref(), flags.deafen.as_deref()),
        (
            Some("You need the Mute Members permission."),
            Some("You need the Deafen Members permission.")
        )
    );
    // The admin may do both to the voice moderator below it.
    let (flags, _) = moderation_of(&gov, Some(&pk(5)), &pk(7));
    assert_eq!((flags.mute, flags.deafen), (None, None));
}
