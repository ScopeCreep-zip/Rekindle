//! A departure's slot heads in the merged state (A/B findings R3-8, F8-1).

use rekindle_types::governance::{AdmissionMode, GovernanceEntry, SlotHead};

use super::super::merge;
use super::{community, head, joined, pk};
use crate::fixture::{cert, op};

/// A departed author's op log stays locatable: its last op (the leave)
/// is published on that record, and frontiers keep naming its ops. The
/// leave's own slot heads are the ones readers keep its history by (A/B
/// finding R3-8); a re-join makes it a member again, not departed.
#[test]
fn a_departed_author_keeps_its_op_log_record() {
    let mut ops = community();
    let heads = vec![SlotHead {
        record_key: "VLD0:ch".into(),
        seq: 6,
        digest: [6; 32],
    }];
    let left = op(
        &pk(4),
        1,
        &[head(&ops)],
        20,
        GovernanceEntry::MemberLeft { heads },
    );
    ops.push(left);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(!state.members.contains_key(&pk(4)), "the member left");
    assert!(state.is_departed(&pk(4)));
    assert_eq!(
        state.op_log_records.get(&pk(4)).map(String::as_str),
        Some("VLD0:m4")
    );
    assert_eq!(
        state.departed_heads.get(&pk(4)),
        Some(
            &[("VLD0:ch".to_string(), [(6, [6; 32])].into_iter().collect())]
                .into_iter()
                .collect()
        ),
        "the leave's heads are recorded"
    );
    let rejoin = op(&pk(4), 2, &[head(&ops)], 30, joined(4, Some("code")));
    ops.push(rejoin);
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    assert!(state.members.contains_key(&pk(4)), "the leaver re-joined");
    assert!(!state.is_departed(&pk(4)));
    // Departing again from another slot adds its head beside the first (F8-1).
    let again = SlotHead {
        record_key: "VLD0:ch".into(),
        seq: 2,
        digest: [2; 32],
    };
    ops.push(op(
        &pk(4),
        3,
        &[head(&ops)],
        40,
        GovernanceEntry::MemberLeft { heads: vec![again] },
    ));
    let state = merge(&cert(&pk(1), AdmissionMode::Open), &ops);
    let kept = state
        .departed_heads
        .get(&pk(4))
        .and_then(|h| h.get("VLD0:ch"));
    assert_eq!(
        kept,
        Some(&[(6, [6; 32]), (2, [2; 32])].into_iter().collect())
    );
}
