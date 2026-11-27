//! The per-target verdicts (plan E3.2 S9) agree with [`validate_write`]
//! for every writer and target of the three-tier fixture, a banned
//! moderator included, so a frontend reading them shows exactly what
//! every reader accepts.

use super::hierarchy::state_with_three_tier_hierarchy;
use super::*;
use crate::validate::target::{may_moderate, outranks, TargetAction};

/// The creator, two admins, a moderator, a member, and a banned moderator.
const PEOPLE: [u8; 6] = [1, 5, 7, 6, 99, 50];

fn state() -> GovernanceState {
    let mut state = state_with_three_tier_hierarchy();
    state
        .role_assignments
        .insert(pseudo(50), HashSet::from([rid(2)]));
    state.bans.insert(pseudo(50));
    state
}

fn entry(action: TargetAction, target: PseudonymKey) -> GovernanceEntry {
    match action {
        TargetAction::Kick => GovernanceEntry::MemberRemoved {
            target,
            reason: None,
            heads: Vec::new(),
            target_seq_frontier: 0,
        },
        TargetAction::Ban => GovernanceEntry::BanEntry {
            target,
            reason: None,
            target_seq_frontier: 0,
        },
        TargetAction::Timeout => GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds: 3600,
            reason: None,
            started_at: 0,
        },
        TargetAction::ManageRoles => GovernanceEntry::RoleUnassignment {
            target,
            role_id: rid(2),
        },
        TargetAction::Mute | TargetAction::Deafen => {
            unreachable!("voice moderation writes no governance entry")
        }
    }
}

/// `ManageRoles` agrees with the validator's `RoleUnassignment` of a role
/// the target cannot self-assign, on every pair but a member and itself
/// (stepping down is the validator's self case; the Host refuses
/// moderating yourself before asking, `capability_words::YOURSELF`).
#[test]
fn manage_roles_is_the_validators_unassignment_verdict() {
    let state = state();
    let mut accepted = 0;
    for w in PEOPLE {
        for t in PEOPLE.into_iter().filter(|t| *t != w) {
            let action = TargetAction::ManageRoles;
            let verdict = validate_write(&pseudo(w), &entry(action, pseudo(t)), &state);
            assert_eq!(
                may_moderate(&pseudo(w), &pseudo(t), action, &state),
                verdict,
                "{w} on {t}"
            );
            accepted += usize::from(verdict);
        }
    }
    assert!(accepted > 0 && accepted < PEOPLE.len() * (PEOPLE.len() - 1));
}

#[test]
fn may_moderate_is_the_validators_verdict() {
    let state = state();
    let mut accepted = 0;
    for action in [TargetAction::Kick, TargetAction::Ban, TargetAction::Timeout] {
        for w in PEOPLE {
            for t in PEOPLE {
                let verdict = validate_write(&pseudo(w), &entry(action, pseudo(t)), &state);
                assert_eq!(
                    may_moderate(&pseudo(w), &pseudo(t), action, &state),
                    verdict,
                    "{action:?} by {w} on {t}"
                );
                accepted += usize::from(verdict);
            }
        }
    }
    // Both verdicts occur, so the agreement is not vacuous.
    assert!(accepted > 0 && accepted < 3 * PEOPLE.len() * PEOPLE.len());
}

#[test]
fn outranks_is_the_validators_unassignment_rank() {
    // Unassigning a role that is not self-assignable is the validator's
    // plain rank check for a writer holding MANAGE_ROLES (everyone here
    // but the member), so the two agree on every other pair.
    let state = state();
    for w in PEOPLE.into_iter().filter(|w| *w != 99) {
        for t in PEOPLE.into_iter().filter(|t| *t != w) {
            let unassign = GovernanceEntry::RoleUnassignment {
                target: pseudo(t),
                role_id: rid(2),
            };
            assert_eq!(
                outranks(&pseudo(w), &pseudo(t), &state),
                validate_write(&pseudo(w), &unassign, &state),
                "{w} over {t}"
            );
        }
    }
    assert!(
        !outranks(&pseudo(5), &pseudo(5), &state),
        "no one outranks themselves"
    );
    assert!(!outranks(&pseudo(99), &pseudo(5), &state));
    assert!(outranks(&pseudo(5), &pseudo(6), &state));
}
