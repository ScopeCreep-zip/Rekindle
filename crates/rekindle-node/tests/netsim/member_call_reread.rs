//! Plan E3.4 S4, netsim B and C's `RequestMEK` variant: a member call
//! routed only by `M`/6, to a member outside our watch ring.
//!
//! B: the member's route dies and its new route reaches its `M`/6 while
//! every hint to us is dropped, so our cache keeps the dead route. Veilid
//! on our node tests each route it imported once it has been idle
//! [`ROUTE_MIN_IDLE`] (veilid-core 0.5.8 `route_spec_store/mod.rs:46`,
//! `route_stats.rs:672-681`); the test of the dead route gets no answer,
//! so once its answer wait runs out Veilid releases it and tells us
//! `dead_remote` (`ping_validator.rs:740-744`): within [`TEST_CADENCE`] of
//! the death. A call after that test: the dead route
//! is refused at import (plan RV1g B1), so the call costs exactly one
//! `M`/6 read and lands on the new route (one call). A call inside the
//! window before that test: it is lost on the dead route, costs exactly
//! one `M`/6 read and lands (two calls: the lost one and the landed one).
//! And a member whose `M`/6 holds no newer route (its DHT is offline as
//! its route dies) is `Unreachable` after exactly one re-read and no
//! further call.
//!
//! C: a member outside the rotator's ring misses a kick's rotation (every
//! send of the rotator dropped), then the rotator heals and the member
//! asks: the answer goes to the route `member_route` takes from `M`/6
//! (`mek_request.rs`, `member_route` then `distribute_mek`), and the
//! member holds the new generation.
//!
//! B's lost-call scenario fails without the rule (a call timeout rereads
//! `M`/6, `RouteCallError::rereads_route`'s `TimedOut` arm): a call into
//! the dead route ends as a `Call` error with no read and no second call.
//! B's tested-route scenario asserts the faithful sequence: Veilid's
//! remote route test released the dead route before the call, the call
//! never lands on it, and exactly one call is made.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::{Fault, RELEASES_PEER_ROUTE, RPC_TIMEOUT};
use rekindle_node::host::adapters::governance::MemberCallError;
use rekindle_records::port::RouteClass;
use rekindle_types::dht_layout::member_record::ROUTES;

use crate::harness::{eventually, Community};

const ANSWERED: &str = "RequestMEK answered";

/// Veilid 0.5.8 `route_spec_store/mod.rs:46` `ROUTE_MIN_IDLE_TIME_MS`: an
/// imported route is tested once no test has proven it for this long.
const ROUTE_MIN_IDLE: Duration = Duration::from_secs(30);

/// The longest a dead imported route stays in Veilid's cache: its next
/// test falls due up to [`ROUTE_MIN_IDLE`] after the last proof (an answer
/// or a passed test, `route_stats.rs:489-498, 672-681`), and Veilid
/// releases it only once that test's answer wait ([`RPC_TIMEOUT`]) has run
/// out (`ping_validator.rs:731-744`), seen on the simulator's 1 s ticks.
/// The netsim's own cadence (`routes_tests.rs`
/// `an_importer_hears_dead_remote_once_the_owner_releases_the_route`).
const TEST_CADENCE: Duration = ROUTE_MIN_IDLE
    .saturating_add(RPC_TIMEOUT)
    .saturating_add(Duration::from_secs(2));

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
    let mut outside = Vec::new();
    for i in 1..community.nodes.len() {
        let record = record_of(community, i).await;
        if !community.sim.watched_subkeys(0, &record).contains(&ROUTES) {
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

/// Node 0's remote route tests that failed and released a peer route
/// (`ping_validator.rs:740-744`).
fn released(community: &Community) -> usize {
    (community.sim.trace().of(community.nodes[0].index))
        .filter(|e| *e == RELEASES_PEER_ROUTE)
        .count()
}

/// Node `x`'s route dies and it allocates a new one, while every hint to
/// node 0 is dropped: node 0's cache keeps the dead route. `settle` is how
/// long the hints stay dropped once the new route exists (its `M`/6 write
/// lands meanwhile).
async fn dead_behind_dropped_hints(community: &Community, x: usize, settle: Duration) {
    let sim = &community.sim;
    let old = community.nodes[x].routes.own_blob(RouteClass::General);
    assert!(old.is_some(), "node {x} holds a route");
    assert_eq!(cached(community, x).await, old, "node 0 cached it");

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
    tokio::time::sleep(settle).await;
    for node in 1..community.nodes.len() {
        sim.fault(Fault::DropFrom { node, per_mille: 0 });
    }
    assert_eq!(cached(community, x).await, old, "the hint was dropped");
}

/// Node 0 calls node `x`: the call lands, after exactly one `M`/6 read and
/// exactly `calls` member calls, and node 0's cache moves to `x`'s route.
async fn call_lands(community: &Community, x: usize, calls: u64, why: &str) {
    let sim = &community.sim;
    let record = record_of(community, x).await;
    let (gets, before) = (sim.app_gets_at(0, &record, ROUTES), member_calls(community));
    let hex = community.pseudonym(x).await;
    let reply = community.nodes[0]
        .host
        .governance()
        .netsim_call_member(&community.anchor, &hex, payload())
        .await;
    assert!(reply.is_ok(), "the call lands on the new route: {reply:?}");
    assert_eq!(
        sim.app_gets_at(0, &record, ROUTES) - gets,
        1,
        "exactly one M/6 read"
    );
    assert_eq!(member_calls(community) - before, calls, "{why}");
    let current = community.nodes[x].routes.own_blob(RouteClass::General);
    assert_eq!(cached(community, x).await, current, "the cache moved");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dropped_routes_hint_costs_one_reread_and_the_call_lands() {
    let (community, outside) = settled().await;
    let x = outside[0];
    let tests_failed = released(&community);
    // Its M/6 write lands, and Veilid on node 0 tests the dead route: node
    // 0's last proof of it may come just before it dies, so the test that
    // releases it ends up to one full cadence later, never sooner.
    dead_behind_dropped_hints(&community, x, TEST_CADENCE).await;
    assert!(
        released(&community) > tests_failed,
        "Veilid's remote route test released the dead route first"
    );
    call_lands(
        &community,
        x,
        1,
        "the dead route is refused at import: only the landed call",
    )
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_call_before_veilid_tests_the_dead_route_is_lost_once_and_lands() {
    let (community, outside) = settled().await;
    let x = outside[0];
    // Under seed 4430 the route dies 5 s past the settle, just after Veilid
    // on node 0 last proved it, and the new route's M/6 write lands within
    // 10 s; the whole lost call then falls inside the window before the
    // next test, and the one re-read is the call's own.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let tests_failed = released(&community);
    dead_behind_dropped_hints(&community, x, Duration::from_secs(10)).await;
    assert_eq!(
        released(&community),
        tests_failed,
        "inside the {ROUTE_MIN_IDLE:?} window: Veilid has not tested the dead route"
    );
    call_lands(&community, x, 2, "the lost call and the landed one").await;
    assert_eq!(
        released(&community),
        tests_failed,
        "the lost call ended before Veilid's test of the dead route"
    );
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
    let tests_failed = released(&community);
    sim.fault(Fault::RouteDeath { node: y });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        released(&community),
        tests_failed,
        "the call starts before the route test"
    );

    let record = record_of(&community, y).await;
    let (gets, calls) = (
        sim.app_gets_at(0, &record, ROUTES),
        member_calls(&community),
    );
    let hex = community.pseudonym(y).await;
    let reply = community.nodes[0]
        .host
        .governance()
        .netsim_call_member(&community.anchor, &hex, payload())
        .await;
    assert_eq!(reply, Err(MemberCallError::Unreachable));
    assert_eq!(
        sim.app_gets_at(0, &record, ROUTES) - gets,
        1,
        "exactly one M/6 read"
    );
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
    assert!(
        eventually(sim, Duration::from_secs(180), || async {
            community_generation(&community, 0).await > before
        })
        .await,
        "the rotator finished its initial push while its sends were dropped"
    );
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
    assert!(
        eventually(sim, Duration::from_secs(60), || async {
            answered() > answered_before
        })
        .await,
        "a RequestMEK was answered and its acknowledgement reached the responder"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn barrier_recovers_a_ring_peers_route_without_a_hint() {
    use rekindle_governance_runtime::unlock_barrier::BarrierPhase;

    let community = Community::of(2, 4431).await;
    let caller = community.nodes[0].host.governance();
    let dee = &community.nodes[1];
    let record = record_of(&community, 1).await;
    let old = dee.routes.own_blob(RouteClass::General).unwrap();
    assert_eq!(
        caller.netsim_barrier_round(&community.anchor).await,
        Some(BarrierPhase::Passed)
    );
    assert_eq!(cached(&community, 1).await, Some(old.clone()));

    // Dee keeps the old route allocated, but its calls time out.
    community.sim.fault(Fault::OutsideDead {
        node: 1,
        class: RouteClass::General,
    });

    // Only M/6 delivers the replacement; the ring watch never sees it.
    let before = community.sim.network_get(&record, ROUTES).unwrap();
    community.sim.fault(Fault::DropValueChanges {
        node: 0,
        key: record.clone(),
        drop: true,
    });
    dee.routes.replace_own_class(RouteClass::General);
    assert!(
        eventually(&community.sim, Duration::from_secs(20), || async {
            community
                .sim
                .network_get(&record, ROUTES)
                .is_some_and(|v| v.seq > before.seq)
        })
        .await,
        "the fresh route landed in M/6"
    );
    assert_eq!(cached(&community, 1).await, Some(old.clone()));
    assert_eq!(
        caller.netsim_barrier_round(&community.anchor).await,
        Some(BarrierPhase::Passed),
        "Dee's frontier provides the one ring answer"
    );
    let current = dee.routes.own_blob(RouteClass::General).unwrap();
    assert_eq!(cached(&community, 1).await, Some(current));
    community.sim.fault(Fault::OutsideDead {
        node: 1,
        class: RouteClass::General,
    });
    assert_eq!(
        caller.netsim_barrier_round(&community.anchor).await,
        None,
        "when M/6 stays unchanged and no ring peer answers, the barrier has no verdict"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn member_call_reread_rejects_a_late_read_and_probe_after_the_successor_dies() {
    use rekindle_presence::community::RouteVersion;
    use tokio::sync::oneshot;

    let community = Community::of(2, 4432).await;
    let caller = community.nodes[0].host.governance();
    let dee = &community.nodes[1];
    let record = record_of(&community, 1).await;
    let previous = community.sim.network_get(&record, ROUTES).unwrap();
    let blob = dee.routes.own_blob(RouteClass::General).unwrap();
    let old = RouteVersion {
        seq: u64::from(previous.seq),
        blob,
    };
    assert!(caller.netsim_observe_route_value(&community.anchor, &record, &previous));

    let (answered, answer) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let probe =
        caller.netsim_delay_probe_answer(&community.anchor, &record, &old, answered, resumed);
    let rotates = async {
        answer.await.expect("the old route answered the probe");
        dee.routes.replace_own_class(RouteClass::General);
        let read = || {
            community
                .sim
                .network_get(&record, ROUTES)
                .filter(|value| value.seq > previous.seq)
        };
        assert!(
            eventually(&community.sim, Duration::from_secs(20), || async {
                read().is_some()
            })
            .await
        );
        assert!(caller.netsim_observe_route_value(&community.anchor, &record, &read().unwrap()));
        let successor = dee.routes.own_blob(RouteClass::General).unwrap();
        assert_ne!(successor, old.blob);
        assert_eq!(cached(&community, 1).await, Some(successor.clone()));
        caller.netsim_declare_member_route_dead(&community.anchor, successor);
        assert!(!caller.netsim_observe_route_value(&community.anchor, &record, &previous));
        resume.send(()).unwrap();
    };
    let (installed, ()) = tokio::join!(probe, rotates);
    assert!(
        !installed,
        "the old probe answer cannot install the older version"
    );
    assert_eq!(cached(&community, 1).await, None);

    let before = community.sim.app_gets_at(0, &record, ROUTES);
    assert_eq!(caller.netsim_barrier_round(&community.anchor).await, None);
    assert_eq!(
        community.sim.app_gets_at(0, &record, ROUTES) - before,
        1,
        "no second read for the same failed device"
    );
}
