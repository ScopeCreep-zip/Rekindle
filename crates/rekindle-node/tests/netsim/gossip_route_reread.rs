//! Plan E3.4 S5 with D8's re-fetch cap (threat model D8, plan VU6): the
//! gossip mesh's re-read of a member's `M`/6 after a failed send. Member
//! `x`'s route dies while its DHT is offline, so its `M`/6 keeps the dead
//! route at the same seq. Veilid on node 0 tests the imported route once it
//! has been idle [`ROUTE_MIN_IDLE`] (veilid-core 0.5.8
//! `route_spec_store/mod.rs:46`), gets no answer and releases it
//! (`ping_validator.rs:740-744`), so every later send to it is refused at
//! import (plan RV1g B1) and the mesh re-reads `M`/6 through its deps
//! (`resolve_peer_route_from_dht`, `HostGovernance::reread_member_route`),
//! which this scenario drives once per failed send. Several failed sends in
//! a row then cost exactly one `M`/6 read: the record's seq cannot change
//! while the re-read found it unchanged, and the dead blob it returns is no
//! route (never cached, never sent to).

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;

use crate::harness::Community;

/// Veilid 0.5.8 `route_spec_store/mod.rs:46` `ROUTE_MIN_IDLE_TIME_MS`.
const ROUTE_MIN_IDLE: Duration = Duration::from_secs(30);

/// Failed sends node 0's mesh makes to `x` while its route is dead and its
/// `M`/6 unchanged.
const SENDS: usize = 5;

/// Node `i`'s member record in node 0's merged roster.
async fn record_of(community: &Community, i: usize) -> String {
    let hex = community.pseudonym(i).await;
    let state = community.nodes[0]
        .host
        .governance()
        .governance_state(&community.anchor)
        .expect("node 0's governance");
    (state.members.iter())
        .find(|(p, _)| p.to_hex() == hex)
        .map(|(_, m)| m.member_record.clone())
        .expect("node i is in node 0's roster")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dead_route_behind_an_unchanged_m6_costs_one_reread_for_many_sends() {
    let community = Community::of(3, 5352).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let (sim, x) = (&community.sim, 1);
    let hex = community.pseudonym(x).await;
    let old = community.nodes[x].routes.own_blob(RouteClass::General);
    assert!(old.is_some(), "node {x} holds a route");
    let in_mesh = || {
        community.nodes[0]
            .host
            .governance()
            .mesh_peers(&community.anchor)
            .into_iter()
            .find(|p| p.pseudonym_key == hex)
            .map(|p| p.route_blob)
    };
    assert_eq!(in_mesh(), old, "node 0's mesh reaches node {x}");

    sim.fault(Fault::DhtOffline {
        node: x,
        offline: true,
    });
    sim.fault(Fault::RouteDeath { node: x });
    // Veilid on node 0 tests the idle imported route and releases it.
    tokio::time::sleep(ROUTE_MIN_IDLE * 2).await;

    let record = record_of(&community, x).await;
    let gets = sim.app_gets(0, &record);
    // Each failed mesh send to `x` re-resolves its route through the gossip
    // deps (`resolve_peer_route_from_dht` -> `reread_member_route`).
    for n in 0..SENDS {
        let fresh = community.nodes[0]
            .host
            .governance()
            .reread_member_route(&community.anchor, &hex, old.as_deref().unwrap())
            .await;
        assert_eq!(fresh, None, "re-read {n}: the dead blob is no route");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let reads = sim.app_gets(0, &record) - gets;
    assert_eq!(
        reads, 1,
        "{SENDS} re-reads of a dead route behind an unchanged M/6 cost {reads} reads; D8 caps it at one"
    );
    let cached = community.nodes[0]
        .host
        .governance()
        .cached_member_route(&community.anchor, &hex);
    assert!(cached.is_none(), "the dead blob is never cached again");
}
