//! Plan E3.2 S7c Verify: per-join cost bounded by the ops above the latest
//! checkpoint, over 64 back-to-back joins at fixed latency (the netsim's
//! default latency model). The creator's genesis holds power ops, so one
//! quiet `SWEEP_PERIOD` after its invite it proposes the height-1
//! checkpoint, usable on its own endorsement (S7c.4); then 64 nodes join
//! back to back through that one invite. Joins are non-power and
//! `PROPOSAL_OPS = 128`, so no later checkpoint is cut and each joiner
//! walks every op above the height-1 cut (its `fetched=` count).
//!
//! **Decided** ("Verify measured 2026-11-18"): "constant in N" means
//! bounded by the ops above the latest checkpoint, as a Matrix faster join
//! costs the state plus the events since it. So each join's simulated time
//! is at most `a + b·(post-cut ops)`, with `a` and `b` from `walk_cost`:
//! - `b`: a post-cut op is read in three fan-outs of sixteen in flight
//!   (the walk above the cut, S7c.5; the bootstrap walk from the inviter's
//!   frontier; the member-card refresh of the first rebuild, S7c.9), each
//!   a sixteenth of one cold fetch (`walk_cost`: 2.16 s);
//! - `a`: the first joins' cost less their ops' share, plus a round trip
//!   per fan-out for a level's last partial batch and half a fetch for
//!   the replicas' jitter (`walk_cost`'s slack).
//!
//! Before S7c.9 the card refresh read one card at a time, and the cost
//! rose ≈ 0.7 s per post-cut op (63.4 → 107.8 s at 6 → 69 ops, over this
//! bound from the 35th join on).

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_governance_runtime::op_walk::WALK_FETCHES_IN_FLIGHT;
use tokio::time::Instant;

use crate::harness::Community;

/// How many joins run back to back.
const JOINS: usize = 64;
/// How many first joins `a` is taken over (their median).
const WINDOW: usize = 8;
/// One cold op fetch, `walk_cost`'s measured figure.
const ONE_FETCH: Duration = Duration::from_millis(2_160);
/// The fan-outs a post-cut op is read in (module doc).
const FAN_OUTS: u32 = 3;

/// The `fetched=` count of node `i`'s "join resumed from a checkpoint" line.
fn resumed_fetch(i: usize) -> Option<usize> {
    let lines = crate::logs::logged(i, &["join resumed from a checkpoint"]);
    let rest = lines.first()?.split("fetched=").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// `b·ops`.
fn ops_share(ops: usize) -> Duration {
    let per_op = ONE_FETCH * FAN_OUTS / u32::try_from(WALK_FETCHES_IN_FLIGHT).expect("16");
    per_op * u32::try_from(ops).expect("a small count")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn sixty_four_joins_cost_at_most_the_ops_above_the_cut() {
    let origin = Instant::now();
    let mut community = Community::create(JOINS + 1, 6464).await;
    let uses = u32::try_from(JOINS).expect("a small count");
    let link = community.invite_for(uses).await;
    // The invite is the creator's last op; the proposal follows a quiet
    // sweep (as in `join_from_hint`).
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    let last = Arc::new(Mutex::new(String::from("no join yet")));
    let done = crate::ring_census::guard(Arc::clone(&last));
    let mut costs = Vec::with_capacity(JOINS);
    let mut fetched = Vec::with_capacity(JOINS);
    let mut starts = Vec::with_capacity(JOINS);
    for i in 1..=JOINS {
        let start = Instant::now();
        starts.push((start - origin).as_secs());
        community.join_with(i, link.clone()).await;
        costs.push(start.elapsed());
        fetched.push(resumed_fetch(i));
        *last.lock() = format!("join {i}: costs {costs:?}, fetched {fetched:?}");
    }
    drop(done);
    writeln!(
        std::io::stderr(),
        "join costs (simulated): {costs:?}\nfetched: {fetched:?}\nstarted at (s): {starts:?}"
    )
    .expect("stderr");
    let ops: Vec<usize> = fetched
        .iter()
        .map(|f| f.expect("every join resumed from a checkpoint"))
        .collect();
    let mut first: Vec<Duration> = (costs.iter().zip(&ops))
        .take(WINDOW)
        .map(|(cost, n)| cost.saturating_sub(ops_share(*n)))
        .collect();
    first.sort_unstable();
    let a = first[WINDOW / 2] + ONE_FETCH * FAN_OUTS + ONE_FETCH / 2;
    for (i, (cost, n)) in costs.iter().zip(&ops).enumerate() {
        let bound = a + ops_share(*n);
        assert!(
            *cost <= bound,
            "join {} cost {cost:?} over a + b·{n} = {bound:?} (a {a:?}): {costs:?}, fetched {ops:?}",
            i + 1
        );
    }
}
