//! `GovernanceState::diff` (plan E3.2 S9): a state against itself is no
//! change, and each membership, ban, role, channel and overwrite change
//! shows in its own category, exactly once.

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry, InviteProof};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};

use super::{GovernanceDiff, MemberChange};
use crate::fixture::{cert, code_key, op, stamp};
use crate::merge::merge;
use crate::state::TimeoutState;
use crate::state::{GovernanceState, MemberState, OverwriteState, PendingMemberState};

fn pk(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn seat(b: u8) -> MemberState {
    MemberState {
        member_record: format!("VLD0:m{b}"),
        member_record_owner: "owner".into(),
        joined: stamp(u64::from(b)),
    }
}

/// A state whose members are `members`, each with an op log record.
fn with(members: &[u8]) -> GovernanceState {
    let mut state = GovernanceState::default();
    for b in members {
        state.members.insert(pk(*b), seat(*b));
        state.op_log_records.insert(pk(*b), format!("VLD0:m{b}"));
    }
    state
}

fn arb_state() -> impl Strategy<Value = GovernanceState> {
    (
        prop::collection::vec(1u8..12, 0..6),
        prop::collection::vec(1u8..12, 0..3),
        prop::collection::vec((1u8..12, 0u8..3), 0..4),
        prop::collection::vec((0u8..3, 1u8..12), 0..3),
    )
        .prop_map(|(members, bans, roles, overwrites)| {
            let mut state = with(&members);
            state.bans = bans.iter().map(|b| pk(*b)).collect();
            for (m, r) in roles {
                state
                    .role_assignments
                    .entry(pk(m))
                    .or_default()
                    .insert(RoleId([r; 16]));
            }
            for (c, t) in overwrites {
                state.overwrites.insert(
                    (ChannelId([c; 16]), format!("{t:064x}")),
                    OverwriteState {
                        target_type: "member".into(),
                        allow: u64::from(t),
                        deny: 0,
                        stamp: stamp(1),
                    },
                );
            }
            state
        })
}

proptest! {
    #[test]
    fn a_state_against_itself_is_no_change(state in arb_state()) {
        prop_assert!(state.diff(&state.clone()).is_empty());
    }

    #[test]
    fn a_new_member_is_exactly_one_joined(state in arb_state(), b in 20u8..40) {
        let mut newer = state.clone();
        newer.members.insert(pk(b), seat(b));
        newer.op_log_records.insert(pk(b), format!("VLD0:m{b}"));
        prop_assert_eq!(
            state.diff(&newer),
            GovernanceDiff {
                membership: vec![MemberChange::Joined { member: pk(b) }],
                ..GovernanceDiff::default()
            }
        );
    }

    #[test]
    fn a_ban_is_exactly_one_banned(state in arb_state(), b in 20u8..40) {
        let older = {
            let mut s = state.clone();
            s.members.insert(pk(b), seat(b));
            s.op_log_records.insert(pk(b), format!("VLD0:m{b}"));
            s
        };
        let mut newer = older.clone();
        newer.members.remove(&pk(b));
        newer.bans.insert(pk(b));
        newer.removed.insert(pk(b), stamp(9));
        newer.removed_by.insert(pk(b), pk(1));
        let diff = older.diff(&newer);
        prop_assert!(diff.bans_changed());
        prop_assert_eq!(
            diff.membership,
            vec![MemberChange::Banned { member: pk(b), by: Some(pk(1)), reason: None }]
        );
    }
}

/// Opening the timeout, lifting it, a pending joiner, a leave, an unban,
/// role, channel-role and overwrite changes each land in their category.
#[test]
fn each_change_lands_in_its_category() {
    let older = with(&[1, 2, 3]);
    let mut newer = older.clone();
    newer.members.remove(&pk(2)); // left: no removal recorded
    newer.pending_members.insert(
        pk(7),
        PendingMemberState {
            member_record: "VLD0:m7".into(),
            member_record_owner: "o".into(),
            has_invite: true,
            stamp: stamp(7),
        },
    );
    newer.timeouts.insert(
        pk(3),
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            reason: Some("calm".into()),
            stamp: stamp(8),
        },
    );
    newer
        .role_assignments
        .insert(pk(1), [RoleId([4; 16])].into());
    newer.overwrites.insert(
        (ChannelId([5; 16]), "everyone".into()),
        OverwriteState {
            target_type: "role".into(),
            allow: 0,
            deny: 1,
            stamp: stamp(9),
        },
    );
    let diff = older.diff(&newer);
    assert_eq!(
        diff.membership,
        vec![
            MemberChange::Left { member: pk(2) },
            MemberChange::TimedOut {
                member: pk(3),
                duration_seconds: 60,
                reason: Some("calm".into()),
                until: 160,
            },
            MemberChange::JoinRequested {
                member: pk(7),
                has_invite: true,
            },
        ]
    );
    assert_eq!(diff.member_roles, vec![pk(1)]);
    assert_eq!(diff.channel_overwrites, vec![ChannelId([5; 16])]);
    assert!(diff.roles.is_empty() && diff.channels.is_empty());
    assert!(!diff.bans_changed());

    let back = newer.diff(&older);
    assert!(back
        .membership
        .contains(&MemberChange::TimeoutLifted { member: pk(3) }));
    let mut unbanned = older.clone();
    unbanned.bans.insert(pk(9));
    assert_eq!(
        unbanned.diff(&older).membership,
        vec![MemberChange::Unbanned { member: pk(9) }]
    );
}

fn joined(b: u8) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: format!("VLD0:m{b}"),
        member_record_owner: "owner".into(),
        invite_proof: (b != 1).then(|| InviteProof {
            code_hash: code_key("code"),
            sig: vec![0; 64],
        }),
        owner_proof: vec![0; 64],
        first_device: None,
    }
}

/// Merging a creator's signed kick over the same ops shows exactly that
/// kick, with its remover and reason.
#[test]
fn a_merged_kick_is_exactly_one_kicked() {
    let cert = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1));
    let invite = GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: code_key("code"),
        max_uses: 5,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    };
    let c1 = op(&pk(1), 1, &[&c0], 1, invite);
    let d0 = op(&pk(3), 0, &[&c1], 5, joined(3));
    let older = merge(&cert, &[c0.clone(), c1.clone(), d0.clone()]);
    assert!(older.members.contains_key(&pk(3)), "member 3 joined");
    let kick = GovernanceEntry::MemberRemoved {
        target: pk(3),
        reason: Some("spam".into()),
        heads: Vec::new(),
        target_seq_frontier: 1,
    };
    let k = op(&pk(1), 2, &[&c1, &d0], 6, kick);
    let newer = merge(&cert, &[c0, c1, d0, k]);
    assert_eq!(
        older.diff(&newer).membership,
        vec![MemberChange::Kicked {
            member: pk(3),
            by: Some(pk(1)),
            reason: Some("spam".into()),
        }]
    );
}
