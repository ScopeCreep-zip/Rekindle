//! A stalled process (plan VU2, Veilid 0.5.8): from 2.5 s on, Veilid sends
//! the unchanged attachment state again (`attachment_manager/mod.rs:557-581`,
//! `:1036-1043`). The node's Host takes the duplicate as no change: it
//! warns of nothing and the pair keeps talking.

use rekindle_netsim::Fault;

use crate::harness::start;
use crate::logs;
use crate::pair::{befriend, talk};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stall_re_sends_the_attachment_and_nothing_breaks() {
    let (sim, mut nodes) = start(2, 95).await;
    for (node, name) in nodes.iter_mut().zip(["Abe", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    let before = logs::counts();
    for _ in 0..3 {
        sim.fault(Fault::Stall { node: 0, ms: 3_000 });
        sim.quiesce().await;
    }
    talk(&sim, &nodes[0], &nodes[1], "after").await;
    assert_eq!(
        logs::counts(),
        before,
        "a re-sent attachment warns of nothing"
    );
}
