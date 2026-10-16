//! `MembershipEvent` samples.

use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::{community, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "joinRequested",
    "joinAccepted",
    "joinRejected",
    "membersRefreshed",
    "memberDiscovered",
    "joinProgress",
    "joined",
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
        MembershipEvent::JoinAccepted { .. } => "joinAccepted",
        MembershipEvent::JoinRejected { .. } => "joinRejected",
        MembershipEvent::MembersRefreshed { .. } => "membersRefreshed",
        MembershipEvent::MemberDiscovered { .. } => "memberDiscovered",
        MembershipEvent::JoinProgress { .. } => "joinProgress",
        MembershipEvent::Joined { .. } => "joined",
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
            "joinAccepted",
            MembershipEvent::JoinAccepted {
                community: community(),
                mek_generation: Some(5),
                slot_index: Some(17),
            },
        ),
        ev(
            "joinRejected",
            MembershipEvent::JoinRejected {
                community: community(),
                reason: text("community is full"),
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
            "joined",
            MembershipEvent::Joined {
                community: community(),
                pseudonym: pk('e'),
                display_name: text("Eve"),
                role_ids: vec![0, 3],
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
            },
        ),
        ev(
            "banned",
            MembershipEvent::Banned {
                community: community(),
                target_pseudonym: pk('e'),
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
                role_ids: vec![0, 3, 7],
            },
        ),
        ev(
            "onboardingCompleted",
            MembershipEvent::OnboardingCompleted {
                community: community(),
                pseudonym: pk('e'),
                role_ids: vec![0, 5],
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
