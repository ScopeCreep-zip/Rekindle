//! Auto-away on the simulated idle port (plan E2.B): a node whose user the
//! scenario sets idle goes Away once its idle time passes the identity's
//! `auto_away_minutes` (10 by default), read on the idle stage's 30 s
//! tick, and is Online again on the first tick after input resumes. The
//! seed, not this machine's input, decides it.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::presence::OwnPresenceView;
use rekindle_types::presence::SessionStatus;

use crate::harness::{self, SimNode};

async fn status(node: &SimNode) -> SessionStatus {
    let view: OwnPresenceView =
        serde_json::from_value(node.ok(IpcRequest::PresenceGet).await).expect("a presence view");
    view.status
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_node_set_idle_goes_away_by_the_auto_away_rule() {
    let (sim, mut nodes) = harness::start(1, 53).await;
    nodes[0].unlock_new_identity("Ida").await;
    let node = &nodes[0];
    assert_eq!(status(node).await, SessionStatus::Online);

    sim.set_idle(0, true);
    let idle_from = tokio::time::Instant::now();
    assert!(
        !harness::eventually(&sim, Duration::from_secs(9 * 60), || async {
            status(node).await == SessionStatus::Away
        })
        .await,
        "away before the 10 minute threshold"
    );
    assert!(
        harness::eventually(&sim, Duration::from_secs(2 * 60), || async {
            status(node).await == SessionStatus::Away
        })
        .await,
        "not away within a tick of the threshold"
    );
    assert!(idle_from.elapsed() >= Duration::from_secs(10 * 60));

    sim.set_idle(0, false);
    assert!(
        harness::eventually(&sim, Duration::from_secs(60), || async {
            status(node).await == SessionStatus::Online
        })
        .await,
        "not back online after input resumed"
    );
}
