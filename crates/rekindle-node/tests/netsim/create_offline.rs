//! A community create while the network cannot take it (finding C-COM1):
//! the Host refuses it at once while detached, and bounds it at
//! `origin::CREATE_DEADLINE` while its DHT calls never return, so the
//! review always resolves; once connected, the same create succeeds.

use std::time::Duration;

use rekindle_governance_runtime::origin::CREATE_DEADLINE;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use crate::harness::{eventually, start, SimNode};

/// How long a reattach may take to be ready again.
const REATTACH: Duration = Duration::from_secs(120);

fn create(name: &str) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityCreate {
            name: name.into(),
            description: String::new(),
            approval_required: false,
        },
    ))
}

/// The communities the node holds.
async fn community_count(node: &SimNode) -> usize {
    let views = node
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    views.as_array().map_or(0, Vec::len)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_create_while_detached_is_refused_at_once_then_succeeds_attached() {
    let (sim, mut nodes) = start(1, 431).await;
    nodes[0].unlock_new_identity("Ada").await;
    sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    // The user acts once the detach reached the node (its status shows it).
    tokio::time::sleep(Duration::from_secs(5)).await;
    let refused = tokio::time::timeout(Duration::from_secs(1), nodes[0].call(create("Early")))
        .await
        .expect("a create while detached resolves at once")
        .expect_err("a create while detached is refused");
    assert!(
        refused.message.contains("Not connected to the network yet"),
        "the Host names why: {}",
        refused.message
    );
    assert_eq!(community_count(&nodes[0]).await, 0, "nothing was created");
    sim.fault(Fault::Detach {
        node: 0,
        detached: false,
    });
    assert!(
        eventually(&sim, REATTACH, || async {
            nodes[0].call(create("Later")).await.is_ok()
        })
        .await,
        "the create succeeds once attached"
    );
    assert_eq!(community_count(&nodes[0]).await, 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_create_whose_writes_never_return_resolves_at_its_deadline() {
    let (sim, mut nodes) = start(1, 433).await;
    nodes[0].unlock_new_identity("Bea").await;
    // Attached and ready, but every DHT call parks (Veilid's set gate
    // holding it, as live).
    sim.fault(Fault::Sleep {
        node: 0,
        asleep: true,
    });
    let answer = tokio::time::timeout(
        CREATE_DEADLINE + Duration::from_secs(10),
        nodes[0].call(create("Stuck")),
    )
    .await
    .expect("the create resolves within its deadline");
    let refused = answer.expect_err("a create past its deadline is an error");
    assert!(
        refused.message.contains("nothing was created"),
        "the Host names the deadline: {}",
        refused.message
    );
    sim.fault(Fault::Sleep {
        node: 0,
        asleep: false,
    });
    assert_eq!(community_count(&nodes[0]).await, 0, "nothing was kept");
    nodes[0].ok(create("Awake")).await;
    assert_eq!(community_count(&nodes[0]).await, 1);
}
