//! `HeadHints` (plan E3.2 S7): read seqs, the frontier claim, the relay
//! rule and pruning.

use std::collections::BTreeSet;

use rekindle_types::dht_layout::member_record::{FRONTIER, GOV_HEAD};

use super::HeadHints;

/// One member's `u64::MAX` hint must not mute a record (register S7l,
/// `evidence/slot-integrity.md` §4.3): the seq kept is the one read
/// from the DHT, so the record's next real advance is still read.
#[test]
fn a_hint_at_u64_max_does_not_mute_the_record() {
    let mut hints = HeadHints::default();
    assert!(hints.worth_reading("M", 2, u64::MAX));
    assert!(hints.record_read("M", 2, 7), "the DHT held seq 7");
    assert!(
        hints.worth_reading("M", 2, 8),
        "the record's real seq 8 is still read"
    );
    assert!(!hints.worth_reading("M", 2, 7), "seq 7 was read");
    assert!(hints.record_read("M", 2, 8));
    assert!(
        !hints.record_read("M", 2, 8),
        "an unmoved read is not acted on"
    );
}

/// A subkey's first value is at seq 0 (Veilid `ValueSeqNum::next`
/// from NONE): a hint at seq 0 for a subkey never read is worth a
/// read, and that read is acted on, so a re-joiner's first post on
/// its fresh member record reaches the members by its own hint
/// (netsim `a_rejoin_missed_by_gossip_converges_by_the_sweep`).
#[test]
fn a_first_value_at_seq_0_is_read() {
    let mut hints = HeadHints::default();
    assert!(hints.worth_reading("M", 4, 0), "subkey 4 was never read");
    assert!(hints.record_read("M", 4, 0), "the first read is acted on");
    assert!(!hints.worth_reading("M", 4, 0), "seq 0 was read");
    assert!(!hints.record_read("M", 4, 0));
    assert!(hints.worth_reading("M", 4, 1));
}

/// A ring change and a hint for one frontier write, interleaved as two
/// concurrent triggers do (the second arrives while the first's DHT reads
/// run): one catch-up, and none again for the seq it read (review S7m
/// 83f7fc6b P1; with check-then-act both would read).
#[test]
fn concurrent_triggers_for_one_write_cost_one_catch_up() {
    let mut hints = HeadHints::default();
    assert!(
        hints.claim_frontier("M", Some(5), true),
        "the ring change claims"
    );
    assert!(
        !hints.claim_frontier("M", Some(5), false),
        "the hint is refused"
    );
    assert!(
        !hints.claim_frontier("M", Some(5), true),
        "so is a second ring change"
    );
    assert_eq!(
        hints.finish_frontier("M", Some(5), false),
        None,
        "nothing to read again"
    );
    assert!(!hints.claim_frontier("M", Some(5), false), "seq 5 was read");
    assert!(hints.claim_frontier("M", Some(6), false), "seq 6 was not");
}

/// A frontier written after the claimed catch-up read is caught up once
/// more, and a catch-up that ended incomplete is retried for a refused
/// trigger; a lying `u64::MAX` hint costs one extra read, then nothing.
#[test]
fn a_write_refused_mid_flight_is_read_again_once() {
    let mut hints = HeadHints::default();
    assert!(hints.claim_frontier("M", Some(5), false));
    assert!(!hints.claim_frontier("M", Some(6), false));
    assert_eq!(hints.finish_frontier("M", Some(5), false), Some(6));
    assert_eq!(hints.finish_frontier("M", Some(6), true), None);

    assert!(hints.claim_frontier("M", Some(7), false));
    assert!(!hints.claim_frontier("M", Some(7), true));
    assert_eq!(
        hints.finish_frontier("M", None, false),
        Some(7),
        "incomplete: read again"
    );
    assert!(!hints.claim_frontier("M", Some(u64::MAX), false));
    assert_eq!(hints.finish_frontier("M", Some(7), true), Some(u64::MAX));
    assert_eq!(
        hints.finish_frontier("M", Some(7), true),
        None,
        "a completed capped attempt releases the claim"
    );
    assert!(
        hints.worth_reading("M", FRONTIER, 8),
        "seq 7 is what was read"
    );
}

/// Only a frontier change that follows a head change is relayed: the
/// author's append (`[2]` then `[3]`, or `[2, 3]` coalesced), never a
/// republish (`[3]` alone) (review S7m 83f7fc6b P2).
#[test]
fn only_a_frontier_after_its_head_is_relayed() {
    let mut hints = HeadHints::default();
    assert!(
        !hints.frontier_follows_head("M", &[FRONTIER], Some(3)),
        "a republish"
    );
    assert!(!hints.frontier_follows_head("M", &[GOV_HEAD], Some(4)));
    assert!(
        hints.frontier_follows_head("M", &[FRONTIER], Some(4)),
        "the append's frontier"
    );
    assert!(
        !hints.frontier_follows_head("M", &[FRONTIER], Some(4)),
        "then a republish"
    );
    assert!(hints.frontier_follows_head("M", &[GOV_HEAD, FRONTIER], Some(5)));
    assert!(!hints.frontier_follows_head("M", &[FRONTIER], Some(5)));
    assert!(!hints.frontier_follows_head("M", &[1, 4], Some(5)));
}

/// LF7c (netsim `ring_frontier`): Veilid reports a value change from each
/// node holding our watch, so the author's append can come as `[2, 3]` and
/// again as `[2]` then `[3]` at the same seqs. Its frontier is relayed
/// once: a change naming a head already seen moves nothing, and a head
/// whose seq is unknown proves no advance.
#[test]
fn a_change_reported_again_relays_its_frontier_once() {
    let mut hints = HeadHints::default();
    assert!(hints.frontier_follows_head("M", &[GOV_HEAD, FRONTIER], Some(11)));
    assert!(!hints.frontier_follows_head("M", &[GOV_HEAD], Some(11)));
    assert!(
        !hints.frontier_follows_head("M", &[FRONTIER], Some(11)),
        "the same append, reported again"
    );
    assert!(!hints.frontier_follows_head("M", &[GOV_HEAD], None));
    assert!(!hints.frontier_follows_head("M", &[FRONTIER], None));
    assert!(!hints.frontier_follows_head("M", &[GOV_HEAD], Some(12)));
    assert!(
        hints.frontier_follows_head("M", &[FRONTIER], Some(12)),
        "the next append"
    );
}

/// A departed member's record is forgotten after a rebuild; a current
/// member's read seqs and moved head are kept.
#[test]
fn a_departed_members_entries_are_pruned() {
    let mut hints = HeadHints::default();
    hints.record_read("Gone", FRONTIER, 4);
    hints.record_read("Kept", FRONTIER, 4);
    hints.frontier_follows_head("Gone", &[GOV_HEAD], Some(1));
    hints.frontier_follows_head("Kept", &[GOV_HEAD], Some(1));
    hints.retain_records(&BTreeSet::from(["Kept".to_owned()]));
    assert!(
        hints.worth_reading("Gone", FRONTIER, 4),
        "Gone's seq was dropped"
    );
    assert!(
        !hints.frontier_follows_head("Gone", &[FRONTIER], Some(1)),
        "and its moved head"
    );
    assert!(!hints.worth_reading("Kept", FRONTIER, 4));
    assert!(hints.frontier_follows_head("Kept", &[FRONTIER], Some(1)));
}

/// Review S7m part 2 P2: a frontier the ring or a hint read is the read
/// the community's sweeps see (one `SeenFrontiers`), and the reverse.
#[test]
fn the_ring_and_the_sweeps_share_one_frontier_read_set() {
    let mut hints = HeadHints::default();
    let sweep = hints.frontiers();
    assert!(hints.claim_frontier("VLD0:m", Some(5), true));
    assert_eq!(hints.finish_frontier("VLD0:m", Some(5), false), None);
    assert!(
        !sweep.unread("VLD0:m", 5),
        "the sweep skips what the ring read"
    );
    sweep.mark_read("VLD0:m", 7);
    assert!(
        !hints.worth_reading("VLD0:m", FRONTIER, 7),
        "the ring skips what the sweep read"
    );
}

#[test]
fn a_lagging_read_keeps_the_original_claim_unread() {
    let mut hints = HeadHints::default();
    assert!(hints.claim_frontier("M", Some(4), true));
    assert!(!hints.claim_frontier("M", Some(5), true));
    assert_eq!(hints.finish_frontier("M", Some(3), false), Some(4));
    assert_eq!(hints.frontiers().get("M"), None);
    assert_eq!(hints.finish_frontier("M", Some(4), true), Some(5));
    assert_eq!(hints.frontiers().get("M"), Some(4));
    assert_eq!(hints.finish_frontier("M", Some(5), true), None);
    assert_eq!(hints.frontiers().get("M"), Some(5));
}
