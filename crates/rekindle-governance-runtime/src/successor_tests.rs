//! Which successors a merge announces (plan E3.2 S7s.3e part 2).

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::SuccessorState;
use rekindle_types::governance::{CommunitySuccessor, OpHash, SuccessorReason};
use rekindle_types::id::PseudonymKey;

use super::newly_applied;
use crate::op_log::{HeldOp, OpFate};

fn held(hash: u8, fate: OpFate) -> HeldOp {
    HeldOp {
        hash: OpHash([hash; 32]),
        author: PseudonymKey([1; 32]),
        author_record: "VLD0:rec".into(),
        seq: u64::from(hash),
        lamport: u64::from(hash),
        issued_at: 0,
        encoded: Vec::new(),
        fate,
    }
}

fn successor(state: &mut GovernanceState, hash: u8, new_anchor: Option<&str>) {
    let row = SuccessorState {
        author: PseudonymKey([1; 32]),
        successor: CommunitySuccessor {
            new_anchor: new_anchor.map(str::to_owned),
            reason: if new_anchor.is_some() {
                SuccessorReason::Recreate
            } else {
                SuccessorReason::Transfer
            },
            transfer_to: None,
            creator_set_hash: None,
            accepts: None,
            links: Vec::new(),
        },
        stamp: OpStamp {
            lamport: u64::from(hash),
            hash: OpHash([hash; 32]),
        },
    };
    state.successors.insert(OpHash([hash; 32]), row);
}

#[test]
fn two_valid_successors_are_both_shown() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:a"));
    successor(&mut state, 2, Some("VLD0:b"));
    // A withdrawal names no anchor; an already applied successor was shown.
    successor(&mut state, 3, None);
    successor(&mut state, 4, Some("VLD0:c"));
    let held = [
        held(1, OpFate::PendingDeps),
        held(2, OpFate::PendingDeps),
        held(3, OpFate::PendingDeps),
        held(4, OpFate::Applied),
    ];
    let anchors: Vec<_> = newly_applied(&held, &state)
        .iter()
        .filter_map(|row| row.successor.new_anchor.clone())
        .collect();
    assert_eq!(anchors, ["VLD0:a", "VLD0:b"]);
}
