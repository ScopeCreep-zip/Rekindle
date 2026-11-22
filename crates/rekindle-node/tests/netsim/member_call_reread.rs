//! Plan E3.4 S4, netsim B and C's `RequestMEK` variant: a member call
//! routed only by `M`/6, to a member outside our watch ring.
//!
//! B: the member's route dies and its new route reaches its `M`/6 while
//! every hint to us is dropped, so our cache keeps the dead route. Our
//! next call to it costs exactly one `M`/6 read and lands on the new route
//! (two calls: the lost one and the landed one); and a member whose `M`/6
//! holds no newer route (its DHT is offline as its route dies) is
//! `Unreachable` after exactly one re-read and no further call.
//!
//! C: a member outside the rotator's ring misses a kick's rotation (every
//! send of the rotator dropped), then the rotator heals and the member
//! asks: the answer goes to the route `member_route` takes from `M`/6
//! (`mek_request.rs`, `member_route` then `distribute_mek`), and the
//! member holds the new generation.
//!
//! B fails without the rule (a call timeout rereads `M`/6,
//! `RouteCallError::rereads_route`'s `TimedOut` arm): a call into the dead
//! route ends as a `Call` error with no read and no second call.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;
use rekindle_node::host::adapters::governance::MemberCallError;
use rekindle_records::port::RouteClass;
use rekindle_types::dht_layout::member_record::SUBKEY_COUNT;

use crate::harness::{eventually, Community};

const ANSWERED: &str = "RequestMEK answered";

/// An opaque payload the member refuses (a NAK): the call still lands.
fn payload() -> Vec<u8> {
    vec![0]
}

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

/// The nodes (but 0) whose member records node 0 does not watch.
async fn outside_ring(community: &Community) -> Vec<usize> {
    let watched = community.sim.watchers_of_dflt(SUBKEY_COUNT);
    let mut outside = Vec::new();
    for i in 1..community.nodes.len() {
        let record = record_of(community, i).await;
        if !watched.get(&record).is_some_and(|w| w.contains(&0)) {
            outside.push(i);
        }
    }
    outside
}

/// A settled community of seven, with node 0's route cache filled.
async fn settled() -> (Community, Vec<usize>) {
    let community = Community::of(7, 4430).await;
    tokio::time::sleep(Duration::from_secs(120)).await;
    let outside = outside_ring(&community).await;
    assert!(
        outside.len() >= 2,
        "the precondition: node 0's ring of 4 leaves members out"
    );
    community.nodes[0]
        .host
        .governance()
        .fill_member_routes(&community.anchor)
        .await;
    (community, outside)
}

/// Node 0's app calls: each is a member call (plan RV1: no own route is
/// self-tested by an app call; Veilid's keepalive tests them).
fn member_calls(community: &Community) -> u64 {
    community.sim.app_calls(0)
}

/// Node 0's cached route of node `i`.
async fn cached(community: &Community, i: usize) -> Option<Vec<u8>> {
    let hex = community.pseudonym(i).await;
    community.nodes[0]
        .host
        .governance()
        .cached_member_route(&community.anchor, &hex)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dropped_routes_hint_costs_one_reread_and_the_call_lands() {
    let (community, outside) = settled().await;
    let (sim, x) = (&community.sim, outside[0]);
    let old = community.nodes[x].routes.own_blob(RouteClass::General);
    assert!(old.is_some(), "node {x} holds a route");
    assert_eq!(cached(&community, x).await, old, "node 0 cached it");

    for node in 1..community.nodes.len() {
        sim.fault(Fault::DropFrom {
            node,
            per_mille: 1000,
        });
    }
    sim.fault(Fault::RouteDeath { node: x });
    let current = || community.nodes[x].routes.own_blob(RouteClass::General);
    assert!(
        eventually(sim, Duration::from_secs(60), || async {
            current().is_some_and(|new| Some(&new) != old.as_ref())
        })
        .await,
        "node {x} allocated a new route"
    );
    // Its M/6 write lands; every hint of it to node 0 is dropped.
    tokio::time::sleep(Duration::from_secs(30)).await;
    for node in 1..community.nodes.len() {
        sim.fault(Fault::DropFrom { node, per_mille: 0 });
    }
    assert_eq!(cached(&community, x).await, old, "the hint was dropped");

    let record = record_of(&community, x).await;
    let (gets, calls) = (sim.app_gets(0, &record), member_calls(&community));
    let hex = community.pseudonym(x).await;
    let reply = community.nodes[0]
        .host
        .governance()
        .netsim_call_member(&community.anchor, &hex, payload())
        .await;
    assert!(reply.is_ok(), "the call lands on the new route: {reply:?}");
    assert_eq!(sim.app_gets(0, &record) - gets, 1, "exactly one M/6 read");
    assert_eq!(
        member_calls(&community) - calls,
        2,
        "the lost call and the landed one"
    );
    assert_eq!(cached(&community, x).await, current(), "the cache moved");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_with_no_newer_route_is_unreachable_after_one_reread() {
    let (community, outside) = settled().await;
    let (sim, y) = (&community.sim, outside[1]);
    let old = cached(&community, y).await;
    assert!(old.is_some(), "node 0 cached node {y}'s route");
    sim.fault(Fault::DhtOffline {
        node: y,
        offline: true,
    });
    sim.fault(Fault::RouteDeath { node: y });
    tokio::time::sleep(Duration::from_secs(30)).await;

    let record = record_of(&community, y).await;
    let (gets, calls) = (sim.app_gets(0, &record), member_calls(&community));
    let hex = community.pseudonym(y).await;
    let reply = community.nodes[0]
        .host
        .governance()
        .netsim_call_member(&community.anchor, &hex, payload())
        .await;
    assert_eq!(reply, Err(MemberCallError::Unreachable));
    assert_eq!(sim.app_gets(0, &record) - gets, 1, "exactly one M/6 read");
    assert_eq!(
        member_calls(&community) - calls,
        1,
        "no call after the re-read"
    );
    assert_eq!(cached(&community, y).await, None, "the dead route is gone");
}

/// The community key generation node `i` holds, or 0 before it holds one.
async fn community_generation(community: &Community, i: usize) -> u64 {
    community
        .mek_list(i)
        .await
        .as_array()
        .into_iter()
        .flatten()
        .filter(|k| k["channel_id"].as_str().is_none_or(str::is_empty))
        .filter_map(|k| k["generation"].as_u64())
        .max()
        .unwrap_or(0)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_that_missed_the_rotation_asks_and_is_answered_by_m6() {
    let (community, outside) = settled().await;
    let (sim, z) = (&community.sim, outside[0]);
    let kicked = *[1, 2, 3, 4, 5, 6]
        .iter()
        .find(|i| **i != z)
        .expect("another member");
    let before = community_generation(&community, z).await;
    let answered = || -> usize {
        (0..7)
            .map(|i| crate::logs::logged(i, &[ANSWERED]).len())
            .sum()
    };
    let answered_before = answered();

    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    let target = community.pseudonym(kicked).await;
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: target,
                reason: None,
            },
        )))
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        community_generation(&community, z).await,
        before,
        "the precondition: node {z} missed the rotation"
    );
    sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    assert!(
        eventually(sim, Duration::from_secs(600), || async {
            community_generation(&community, z).await > before
        })
        .await,
        "node {z} holds the rotated key"
    );
    assert!(answered() > answered_before, "a RequestMEK was answered");
}
