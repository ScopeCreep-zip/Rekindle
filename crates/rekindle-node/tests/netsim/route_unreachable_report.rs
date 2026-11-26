//! Route evidence source 7, an unreachable report (plan RV2). Ann befriends
//! Bea and Cat, who are not friends, and the three hold a group. Then Cat's
//! General route stops taking traffic from outside while Cat's own
//! loopback test of it still passes ([`Fault::OutsideDead`]), and Cat
//! expects nothing from anyone, so none of Cat's own evidence fires. Bea
//! posts: its hint to Cat really calls Cat's General route (`group_dm`
//! `content::spawn_hints` → `dm::send::send_to_peer` → `route_send`'s
//! `send_direct`, an `app_message` on Cat's imported blob). Veilid on Bea's
//! node tests that import, gets no answer and releases it (veilid-core
//! 0.5.8 `ping_validator.rs:732-745`), and Bea's Rekindle forgets the blob
//! (`subscriptions::dispatch::forget_dead_remote`). Bea posts again, as the
//! GI1 caller kept re-dialling: the hint re-resolves the route
//! (`route_send` → `resolve_and_send` → `route::fetch_from_profile`), the
//! re-read returns the same declared-dead blob, so Bea queues one
//! `GroupRouteReport` naming it on her group chain, which Cat watches; Cat
//! rotates on it, republishes, and Bea's posts land.
//!
//! Fails without the report's enqueue (`rekindle-transport`
//! `dm::route_report::report_if_dead`): Bea queues no report.

use std::time::Duration;

use rekindle_netsim::{Fault, RELEASES_PEER_ROUTE, RPC_TIMEOUT};
use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{PUBLISH_DELAY, PUBLISH_GAP, REFETCH_GAP};

use crate::group_dm::{reads, send, three_members};
use crate::harness::{eventually, start};

/// Veilid 0.5.8 `route_spec_store/mod.rs:46` `ROUTE_MIN_IDLE_TIME_MS`.
const ROUTE_MIN_IDLE: Duration = Duration::from_secs(30);

/// The reason Cat's rotation logs (`rekindle-transport` `liveness::owner`).
const REPORTED: &str = "a peer reported our General route unreachable";

/// What Bea logs when she queues a report (`dm::route_report::enqueue`).
const QUEUED: &str = "reported the peer's General route unreachable";

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_route_dead_from_outside_is_rotated_on_a_members_report() {
    let (sim, mut nodes) = start(3, 317).await;
    let gid = three_members(&sim, &mut nodes).await;
    let released = || {
        sim.trace()
            .of(nodes[1].index)
            .filter(|e| *e == RELEASES_PEER_ROUTE)
            .count()
    };
    let before_released = released();
    let before = sim.route_blobs(2);
    assert!(!before.is_empty(), "Cat has a General route");
    sim.fault(Fault::OutsideDead {
        node: nodes[2].index,
        class: RouteClass::General,
    });
    let body = "Bea to a route dead from outside";
    send(&nodes[1], &gid, body).await;
    assert!(
        eventually(&sim, REFETCH_GAP + ROUTE_MIN_IDLE + RPC_TIMEOUT * 2, || {
            std::future::ready(released() > before_released)
        })
        .await,
        "Bea's Veilid tests Cat's route, gets no answer and releases it"
    );
    // The second post re-resolves Cat's route from Cat's profile, which
    // still names the blob Veilid declared dead.
    let again = "Bea again, re-dialling";
    send(&nodes[1], &gid, again).await;
    assert!(
        eventually(&sim, REFETCH_GAP + RPC_TIMEOUT, || {
            std::future::ready(!crate::logs::logged(nodes[1].index, &[QUEUED]).is_empty())
        })
        .await,
        "Bea queues a report of Cat's dead route"
    );
    assert!(
        eventually(&sim, REFETCH_GAP * 2 + PUBLISH_DELAY + PUBLISH_GAP, || {
            std::future::ready(!crate::logs::logged(nodes[2].index, &[REPORTED]).is_empty())
        })
        .await,
        "Cat rotates its General route on the report"
    );
    assert!(
        eventually(&sim, PUBLISH_DELAY + PUBLISH_GAP * 2, || {
            let now = sim.route_blobs(2);
            let replaced = !now.is_empty() && now.iter().all(|b| !before.contains(b));
            async move { replaced }
        })
        .await,
        "Cat's replacement route is up"
    );
    for body in [body, again] {
        assert!(
            eventually(&sim, Duration::from_secs(120), || reads(
                &nodes[2], &gid, body
            ))
            .await,
            "Bea's post {body:?} lands at Cat"
        );
    }
    assert_eq!(
        crate::logs::logged(nodes[1].index, &[QUEUED]).len(),
        1,
        "one report per dead blob"
    );
}
