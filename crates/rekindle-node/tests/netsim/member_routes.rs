//! Plan E3.4 S4, member routes on `M`/6: when a member's route dies and its
//! route publisher writes the new one to its member record's `M`/6, every
//! other member's route cache holds the new route within one gossip round:
//! a ring watcher's by its watch, a non-ring member's by the ring watcher's
//! `HeadAdvanced` hint for subkey 6, and neither rebuilds governance. The
//! cache is what every Host call to a member uses (MEK answering and
//! distribution, the key transfer, files, voice), on both modes.
//!
//! Fails with subkey 6 outside `RELAYED_SUBKEYS`: the ring watcher's change
//! is dropped as naming no relayed subkey and no hint is gossiped, so the
//! caches keep the dead route.

use std::time::Duration;

use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;

use crate::harness::{eventually, Community};

const REBUILD: &str = "ring member's frontier changed";

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_replaced_route_reaches_every_members_cache_without_a_rebuild() {
    let community = Community::of(3, 4410).await;
    let member = community.pseudonym(2).await;
    let cached = |reader: usize| {
        community.nodes[reader]
            .host
            .governance()
            .cached_member_route(&community.anchor, &member)
    };
    let current = || community.nodes[2].routes.own_blob(RouteClass::General);
    let old = current().expect("node 2 holds a route");
    // The ring's first reads fill the caches with the route of the day.
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            cached(0).as_ref() == Some(&old) || cached(1).as_ref() == Some(&old)
        })
        .await,
        "a ring watcher cached node 2's route"
    );
    // The community settles (the joins' ops and frontiers land) before the
    // rebuilds are counted.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    let rebuilds = |node: usize| crate::logs::logged(node, &[REBUILD]).len();
    let before = [rebuilds(0), rebuilds(1)];

    community.sim.fault(Fault::RouteDeath { node: 2 });
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            current().is_some_and(|new| new != old)
        })
        .await,
        "node 2 allocated a new route"
    );
    for reader in [0, 1] {
        assert!(
            eventually(&community.sim, Duration::from_secs(10), || async {
                cached(reader).is_some() && cached(reader) == current()
            })
            .await,
            "node {reader} caches node 2's new route from M/6"
        );
    }
    assert_eq!(
        [rebuilds(0), rebuilds(1)],
        before,
        "a route change never rebuilds governance"
    );
}
