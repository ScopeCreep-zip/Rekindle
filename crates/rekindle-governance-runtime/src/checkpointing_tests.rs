//! The round clock (plan E3.2 S7c.4): our round-r endorsement counts as
//! timed out at the first tick one `SWEEP_PERIOD` after we wrote it, and an
//! endorsement first seen at a tick (written before a restart) waits one
//! more tick.

use rekindle_governance::fixture::op;
use rekindle_types::governance::{Checkpoint, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use super::CheckpointClock;

fn endorsement(me: &PseudonymKey, round: u32) -> rekindle_governance::op::VerifiedOp {
    op(
        me,
        u64::from(round),
        &[],
        u64::from(round),
        GovernanceEntry::Checkpoint(Checkpoint {
            height: 1,
            round,
            prev: None,
            cut: Vec::new(),
            frontier_hash: [0; 32],
            dir_root: [0; 32],
            dir_size: 0,
            state_root: [0; 32],
            mek_generation: 1,
            eligible_set_hash: [0; 32],
            pages: Vec::new(),
        }),
    )
}

#[test]
fn our_endorsement_times_out_one_sweep_after_we_wrote_it() {
    let me = PseudonymKey([1; 32]);
    let ops = vec![endorsement(&me, 0)];
    let mut written = CheckpointClock {
        waiting: Some(((1, 0), true)),
        ..CheckpointClock::default()
    };
    assert!(written.timed_out(&me, &ops, 1), "one tick after the write");

    let mut restarted = CheckpointClock::default();
    assert!(
        !restarted.timed_out(&me, &ops, 1),
        "first seen at this tick"
    );
    assert!(restarted.timed_out(&me, &ops, 1), "one tick later");

    let opened = vec![endorsement(&me, 0), endorsement(&me, 1)];
    assert!(
        !restarted.timed_out(&me, &opened, 1),
        "round 1 starts its own wait"
    );
    assert!(
        !CheckpointClock::default().timed_out(&me, &[], 1),
        "none of ours"
    );
}
