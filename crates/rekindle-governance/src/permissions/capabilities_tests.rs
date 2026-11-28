//! `denied_capabilities`: the real cause per capability, never a generic one.

use rekindle_types::dto::community_capability::CommunityCapability as C;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::{ADMINISTRATOR, DEFAULT_EVERYONE, DEFINED, MANAGE_CHANNELS};

use super::{denied_capabilities, CapabilityDenial, PERMISSION_NAMES};
use crate::op::OpStamp;
use crate::state::{GovernanceState, RoleState, TimeoutState};

fn key(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn role(name: &str, permissions: u64, position: u32) -> RoleState {
    RoleState {
        name: name.into(),
        permissions,
        position,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: OpStamp::MIN,
    }
}

fn state() -> GovernanceState {
    let mut s = GovernanceState::default();
    s.roles
        .insert(RoleId([0; 16]), role("@everyone", DEFAULT_EVERYONE, 0));
    s.roles
        .insert(RoleId([5; 16]), role("Builders", MANAGE_CHANNELS, 1));
    s
}

fn denial(s: &GovernanceState, who: u8, cap: C, now: u64) -> Option<CapabilityDenial> {
    denied_capabilities(&key(who), s, now)
        .into_iter()
        .find_map(|(c, d)| (c == cap).then_some(d))
}

#[test]
fn a_plain_member_lacks_manage_channels_and_the_role_names_it() {
    let mut s = state();
    assert_eq!(
        denial(&s, 1, C::ManageChannels, 100),
        Some(CapabilityDenial::Missing {
            permission: "Manage Channels"
        })
    );
    assert_eq!(
        denial(&s, 1, C::OfferSuccessor, 100),
        Some(CapabilityDenial::NotCreator)
    );
    s.role_assignments
        .entry(key(1))
        .or_default()
        .insert(RoleId([5; 16]));
    let denied = denied_capabilities(&key(1), &s, 100);
    for cap in [C::ManageChannels, C::SetTopic, C::ManageServers] {
        assert!(
            denied.iter().all(|(c, _)| *c != cap),
            "{cap:?} still denied"
        );
    }
    assert_eq!(
        denial(&s, 1, C::ManageRoles, 100),
        Some(CapabilityDenial::Missing {
            permission: "Manage Roles"
        })
    );
}

#[test]
fn a_timed_out_member_is_told_the_timeout_only_where_it_is_the_cause() {
    let mut s = state();
    s.role_assignments
        .entry(key(1))
        .or_default()
        .insert(RoleId([5; 16]));
    s.timeouts.insert(
        key(1),
        TimeoutState {
            duration_seconds: 600,
            started_at: 1_000,
            reason: None,
            stamp: OpStamp::MIN,
        },
    );
    assert_eq!(
        denial(&s, 1, C::ManageChannels, 1_100),
        Some(CapabilityDenial::TimedOut {
            until_secs: 1_600,
            permission: "Manage Channels"
        })
    );
    // Not held even untimed: the missing permission is the cause.
    assert_eq!(
        denial(&s, 1, C::BanMembers, 1_100),
        Some(CapabilityDenial::Missing {
            permission: "Ban Members"
        })
    );
    // Run out: held again.
    assert_eq!(denial(&s, 1, C::ManageChannels, 1_600), None);
}

#[test]
fn a_creator_holds_everything_and_an_administrator_all_but_the_seat() {
    let mut s = state();
    s.creators.insert(key(1));
    assert!(denied_capabilities(&key(1), &s, 100).is_empty());
    s.roles
        .insert(RoleId([9; 16]), role("Admin", ADMINISTRATOR, 2));
    s.role_assignments
        .entry(key(2))
        .or_default()
        .insert(RoleId([9; 16]));
    assert_eq!(
        denied_capabilities(&key(2), &s, 100),
        vec![(C::OfferSuccessor, CapabilityDenial::NotCreator)]
    );
}

/// Per target (plan E3.2 S9): a moderator ranked below a member is refused
/// by rank, and may act on a member ranked below it.
#[test]
fn a_lower_moderator_is_refused_by_rank_above_and_allowed_below() {
    use crate::validate::target::{moderation_refusal, TargetAction, TargetRefusal};
    use rekindle_types::permissions::{KICK_MEMBERS, MANAGE_ROLES};
    let mut s = state();
    s.roles.insert(
        RoleId([6; 16]),
        role("Mods", KICK_MEMBERS | MANAGE_ROLES, 2),
    );
    s.roles.insert(RoleId([7; 16]), role("Admins", 0, 3));
    let assign = |s: &mut GovernanceState, who: u8, r: u8| {
        s.role_assignments
            .entry(key(who))
            .or_default()
            .insert(RoleId([r; 16]));
    };
    assign(&mut s, 1, 6); // the moderator
    assign(&mut s, 2, 7); // above it
    assign(&mut s, 3, 5); // below it
    for action in [TargetAction::Kick, TargetAction::ManageRoles] {
        assert_eq!(
            moderation_refusal(&key(1), &key(2), action, &s),
            Some(TargetRefusal::Rank)
        );
        assert_eq!(moderation_refusal(&key(1), &key(3), action, &s), None);
    }
    assert_eq!(
        moderation_refusal(&key(1), &key(3), TargetAction::Ban, &s),
        Some(TargetRefusal::Missing {
            permission: "Ban Members"
        })
    );
}

/// Voice moderation per target (plan E3.2 S9): server mute and deafen need
/// their own permission and a target ranked strictly below the writer.
#[test]
fn mute_and_deafen_need_their_permission_and_a_lower_target() {
    use crate::validate::target::{moderation_refusal, TargetAction, TargetRefusal};
    use rekindle_types::permissions::{CONNECT, DEAFEN_MEMBERS, KICK_MEMBERS, MUTE_MEMBERS};
    let mut s = state();
    s.roles.insert(
        RoleId([6; 16]),
        role("Voice", CONNECT | MUTE_MEMBERS | DEAFEN_MEMBERS, 2),
    );
    s.roles.insert(RoleId([7; 16]), role("Admins", 0, 3));
    s.roles
        .insert(RoleId([8; 16]), role("Kickers", KICK_MEMBERS, 2));
    let assign = |s: &mut GovernanceState, who: u8, r: u8| {
        s.role_assignments
            .entry(key(who))
            .or_default()
            .insert(RoleId([r; 16]));
    };
    assign(&mut s, 1, 6); // the voice moderator
    assign(&mut s, 2, 7); // above it
    assign(&mut s, 3, 5); // below it
    assign(&mut s, 4, 8); // a kicker without voice permissions
    for (action, permission) in [
        (TargetAction::Mute, "Mute Members"),
        (TargetAction::Deafen, "Deafen Members"),
    ] {
        assert_eq!(
            moderation_refusal(&key(1), &key(2), action, &s),
            Some(TargetRefusal::Rank)
        );
        assert_eq!(moderation_refusal(&key(1), &key(3), action, &s), None);
        assert_eq!(
            moderation_refusal(&key(4), &key(3), action, &s),
            Some(TargetRefusal::Missing { permission })
        );
    }
}

/// The channel-scoped controls are judged with the channel's overwrites
/// (as `AdminDelete` is validated): a moderator whose permission an
/// overwrite takes away is told the overwrite, one without it the
/// permission, a timed-out one the timeout.
#[test]
fn a_channel_capability_names_the_overwrite_the_permission_or_the_timeout() {
    use rekindle_types::dto::community_capability::ChannelCapability as Ch;
    use rekindle_types::governance::GovernanceEntry;
    use rekindle_types::id::ChannelId;
    use rekindle_types::permissions::{
        BYPASS_SLOWMODE, MANAGE_MESSAGES, MENTION_EVERYONE, REQUEST_TO_SPEAK, USE_SOUNDBOARD,
    };

    use super::denied_in_channel;
    let room = ChannelId([9; 16]);
    let mut s = state();
    s.roles.get_mut(&RoleId::EVERYONE).unwrap().permissions |=
        BYPASS_SLOWMODE | REQUEST_TO_SPEAK | MENTION_EVERYONE;
    s.roles.insert(
        RoleId([6; 16]),
        role("Mods", MANAGE_MESSAGES | USE_SOUNDBOARD, 2),
    );
    s.role_assignments
        .entry(key(2))
        .or_default()
        .insert(RoleId([6; 16]));
    let deny = GovernanceEntry::PermissionOverwrite {
        channel_id: room,
        target_type: "role".into(),
        target_id: RoleId([6; 16]).to_hex(),
        allow: 0,
        deny: MANAGE_MESSAGES,
    };
    crate::merge::apply_entry(&key(9), &deny, crate::fixture::stamp(1), &mut s);
    let denied =
        |s: &GovernanceState, who: u8, now: u64| denied_in_channel(&key(who), &room, s, now);
    assert_eq!(
        denied(&s, 2, 100),
        vec![(
            Ch::ManageMessages,
            CapabilityDenial::ChannelDenied {
                permission: "Manage Messages"
            }
        )]
    );
    assert_eq!(
        denied(&s, 1, 100),
        vec![
            (
                Ch::ManageMessages,
                CapabilityDenial::Missing {
                    permission: "Manage Messages"
                }
            ),
            (
                Ch::UseSoundboard,
                CapabilityDenial::Missing {
                    permission: "Use Soundboard"
                }
            ),
        ]
    );
    s.timeouts.insert(
        key(2),
        TimeoutState {
            started_at: 50,
            duration_seconds: 100,
            reason: None,
            stamp: OpStamp::MIN,
        },
    );
    assert!(denied(&s, 2, 100).contains(&(
        Ch::UseSoundboard,
        CapabilityDenial::TimedOut {
            until_secs: 150,
            permission: "Use Soundboard"
        }
    )));
}

#[test]
fn each_defined_permission_has_one_name() {
    let mut named = 0;
    for &(bit, name) in PERMISSION_NAMES {
        assert!(bit.is_power_of_two());
        assert!(!name.is_empty());
        assert_eq!(named & bit, 0, "duplicate bit for {name}");
        named |= bit;
    }
    assert_eq!(named, DEFINED);
}
