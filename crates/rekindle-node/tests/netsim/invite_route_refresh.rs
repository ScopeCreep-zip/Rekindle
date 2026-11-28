//! An invite outlives its inviter's route (plan E3.2 S7c.8 item (4)). The
//! creator mints an invite, waits for its height-1 checkpoint to become
//! usable, then its routes die (`Fault::RouteDeath`: only the owner hears,
//! importers are never told) and it allocates a new one. The joiner reads
//! the inviter's current route from its member record's `M`/6 when it uses
//! the invite, so its hint request and bootstrap call reach the inviter
//! over the new route, and the join resumes from the checkpoint.
//!
//! Fails with the route frozen into the invite's secrets at minting: the
//! hint request goes into the dead route, no hint comes back, and the join
//! walks in full (no "join resumed from a checkpoint").

use std::time::Duration;

use rekindle_netsim::Fault;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_reaches_its_inviter_after_the_inviters_route_was_replaced() {
    let mut community = Community::create(2, 211).await;
    let link = community.invite().await;
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    community.sim.fault(Fault::RouteDeath { node: 0 });
    // The new route is allocated and written to `M`/6.
    tokio::time::sleep(Duration::from_secs(60)).await;
    community.join_with(1, link).await;
    let resumed = crate::logs::logged(1, &["join resumed from a checkpoint"]);
    assert_eq!(
        resumed.len(),
        1,
        "the hint request reached the inviter's new route: {resumed:?}"
    );
    assert!(crate::logs::warned("roster checksum").is_empty());
}
