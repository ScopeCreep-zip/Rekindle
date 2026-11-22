//! `MembershipEvent` samples.

use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::{community, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "joinRequested",
    "membersRefreshed",
    "memberDiscovered",
    "joinProgress",
    "joined",
    "selfJoined",
    "memberCardChanged",
    "left",
    "removed",
    "kicked",
    "banned",
    "unbanned",
    "timedOut",
    "timeoutRemoved",
    "timeoutStatusChanged",
    "rolesChanged",
    "onboardingCompleted",
    "onboardingAnswersSubmitted",
];

pub fn variant_name(e: &MembershipEvent) -> &'static str {
    match e {
        MembershipEvent::JoinRequested { .. } => "joinRequested",
        MembershipEvent::MembersRefreshed { .. } => "membersRefreshed",
        MembershipEvent::MemberDiscovered { .. } => "memberDiscovered",
        MembershipEvent::JoinProgress { .. } => "joinProgress",
        MembershipEvent::Joined { .. } => "joined",
        MembershipEvent::SelfJoined { .. } => "selfJoined",
        MembershipEvent::MemberCardChanged { .. } => "memberCardChanged",
        MembershipEvent::Left { .. } => "left",
        MembershipEvent::Removed { .. } => "removed",
        MembershipEvent::Kicked { .. } => "kicked",
        MembershipEvent::Banned { .. } => "banned",
        MembershipEvent::Unbanned { .. } => "unbanned",
        MembershipEvent::TimedOut { .. } => "timedOut",
        MembershipEvent::TimeoutRemoved { .. } => "timeoutRemoved",
        MembershipEvent::TimeoutStatusChanged { .. } => "timeoutStatusChanged",
        MembershipEvent::RolesChanged { .. } => "rolesChanged",
        MembershipEvent::OnboardingCompleted { .. } => "onboardingCompleted",
        MembershipEvent::OnboardingAnswersSubmitted { .. } => "onboardingAnswersSubmitted",
    }
}

fn ev(name: &str, e: MembershipEvent) -> Sample {
    (
        format!("membership.{name}"),
        SubscriptionEvent::Membership(e),
    )
}

pub fn samples() -> Vec<Sample> {
    let mut all = joining();
    all.extend(moderation());
    all.extend(roles_and_onboarding());
    all
}

fn joining() -> Vec<Sample> {
    vec![
        ev(
            "joinRequested",
            MembershipEvent::JoinRequested {
                community: community(),
                pseudonym: pk('e'),
                display_name: text("Eve"),
                has_invite: true,
            },
        ),
        ev(
            "membersRefreshed",
            MembershipEvent::MembersRefreshed {
                community: community(),
            },
        ),
        ev(
            "memberDiscovered",
            MembershipEvent::MemberDiscovered {
                community: community(),
                pseudonym: pk('e'),
                display_name: text("Eve"),
                subkey_index: 17,
            },
        ),
        ev(
            "joinProgress",
            MembershipEvent::JoinProgress {
                community: community(),
                stage: text("slotClaim"),
                status: text("done"),
            },
        ),
        ev(
            "selfJoined",
            MembershipEvent::SelfJoined {
                community: community(),
                name: text("Frag Night"),
            },
        ),
        ev(
            "joined",
            MembershipEvent::Joined {
                community: community(),
                pseudonym: pk('e'),
                display_name: Some(text("Eve")),
                role_ids: vec!["00".repeat(16), "03".repeat(16)],
            },
        ),
        ev(
            "memberCardChanged",
            MembershipEvent::MemberCardChanged {
                community: community(),
                pseudonym: pk('e'),
                display_name: text("Eve"),
            },
        ),
        ev(
            "left",
            MembershipEvent::Left {
                community: community(),
                pseudonym: pk('e'),
            },
        ),
    ]
}

fn moderation() -> Vec<Sample> {
    vec![
        ev(
            "removed",
            MembershipEvent::Removed {
                community: community(),
                pseudonym: pk('e'),
            },
        ),
        ev(
            "kicked",
            MembershipEvent::Kicked {
                community: community(),
                target_pseudonym: pk('e'),
                removed_by: None,
                removed_by_name: None,
                reason: None,
            },
        ),
        ev(
            "banned",
            MembershipEvent::Banned {
                community: community(),
                target_pseudonym: pk('e'),
                removed_by: Some(pk('f')),
                removed_by_name: Some("Mod".into()),
                reason: Some(text("spam")),
            },
        ),
        ev(
            "unbanned",
            MembershipEvent::Unbanned {
                community: community(),
                target_pseudonym: pk('e'),
            },
        ),
        ev(
            "timedOut",
            MembershipEvent::TimedOut {
                community: community(),
                target_pseudonym: pk('e'),
                duration_seconds: 600,
                reason: Some(text("spamming")),
            },
        ),
        ev(
            "timeoutRemoved",
            MembershipEvent::TimeoutRemoved {
                community: community(),
                target_pseudonym: pk('e'),
            },
        ),
        ev(
            "timeoutStatusChanged",
            MembershipEvent::TimeoutStatusChanged {
                community: community(),
                pseudonym: pk('e'),
                timeout_until: Some(1_760_000_600_000),
            },
        ),
    ]
}

fn roles_and_onboarding() -> Vec<Sample> {
    vec![
        ev(
            "rolesChanged",
            MembershipEvent::RolesChanged {
                community: community(),
                pseudonym: pk('e'),
                role_ids: vec!["00".repeat(16), "03".repeat(16), "07".repeat(16)],
            },
        ),
        ev(
            "onboardingCompleted",
            MembershipEvent::OnboardingCompleted {
                community: community(),
                pseudonym: pk('e'),
                role_ids: vec!["00".repeat(16), "05".repeat(16)],
            },
        ),
        ev(
            "onboardingAnswersSubmitted",
            MembershipEvent::OnboardingAnswersSubmitted {
                community: community(),
                sender_pseudonym: pk('e'),
                answer_count: 3,
            },
        ),
    ]
}
