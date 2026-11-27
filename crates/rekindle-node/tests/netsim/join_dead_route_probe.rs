//! Diag 2026-11-26 RC2/RC6 (plan RH1r-c S2/S3 on the bootstrap path;
//! inventory item 25; E3.2 join step 5, D22). The joiner's call into the
//! inviter's live route gets no answer in time ([`Fault::CallStalls`], the
//! call stuck past the scenario), so the bootstrap exchange is queued, and
//! its single remote test ping of that route is lost on the slow round
//! trip ([`Fault::RemoteTestLost`]), so Veilid removes the import and tells
//! the joiner alone `dead_remote` (0.5.8 `ping_validator.rs:741-744`). The
//! inviter's loopback passes, so it never republishes: only the failure
//! cache's expiry (120 s) or a probe can revive the blob on the joiner.
//! With the probe, the bootstrap attempt whose `M`/6 read finds the same
//! blob at the same seq one `REFETCH_GAP` after the death probes it, the
//! probe answers, and the same attempt's exchange lands: the inviter merges
//! the joiner's `MemberJoined` well before the failure cache would have let
//! the blob be called again.
//!
//! Fails without RC2 (`DeadRoutes::observe` never asking for a probe): the
//! joiner refuses the blob until `FAILURE_CACHE` expires.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::{Fault, RELEASES_PEER_ROUTE};
use rekindle_records::port::RouteClass;
use rekindle_route::liveness::{FAILURE_CACHE, REFETCH_GAP};

use crate::harness::{eventually, Community};
use crate::join_alone::holds_member;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_live_inviter_route_whose_test_ping_was_lost_is_probed_and_the_join_lands() {
    let mut community = Community::create(2, 4311).await;
    let link = community.invite().await;
    community.sim.quiesce().await;
    let (inviter, joiner) = (community.nodes[0].index, community.nodes[1].index);
    let released = || {
        (community.sim.trace().of(joiner))
            .filter(|e| *e == RELEASES_PEER_ROUTE)
            .count()
    };
    let before = released();
    community.sim.fault(Fault::CallStalls {
        node: inviter,
        stall: Some(Duration::from_secs(24 * 3600)),
    });
    community.sim.fault(Fault::RemoteTestLost {
        importer: joiner,
        owner: inviter,
        class: RouteClass::General,
    });
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    let pseudonym = community.pseudonym(1).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || {
            std::future::ready(released() > before)
        })
        .await,
        "the joiner's one test ping of the inviter's route is lost: Veilid releases the import"
    );
    // The route answers calls again; the inviter never republishes it.
    community.sim.fault(Fault::CallStalls {
        node: inviter,
        stall: None,
    });
    // One re-fetch gap, the bootstrap backoff step that reaches it, and the
    // probe and exchange round trips: inside the failure cache.
    let bound = REFETCH_GAP + Duration::from_secs(40);
    assert!(bound < FAILURE_CACHE);
    assert!(
        eventually(&community.sim, bound, || async {
            holds_member(&community, 0, &pseudonym)
        })
        .await,
        "the probe revives the inviter's route and the inviter merges the joiner's MemberJoined"
    );
}
