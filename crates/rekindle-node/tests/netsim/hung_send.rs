//! An `app_message` that is never dispatched is ended by our own deadline
//! (review of live finding 78, M1): Veilid bounds no stage of a send
//! (destination, render, the first hop's `send_envelope`), so a hint to a
//! hung route would hold whatever awaits it. The transport's port bound
//! (`rekindle_transport::broadcast::deadline::app_message_deadline`) makes it
//! a send error: the outbox pump goes on to the next row and to other
//! peers, and the message still arrives through its chain.

use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::pair::{befriend, holds_message, key, send, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_hint_to_a_hung_route_blocks_neither_later_sends_nor_the_pump() {
    let (sim, mut nodes) = start(3, 79).await;
    for (node, name) in nodes.iter_mut().zip(["Ida", "Jon", "Kim"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    befriend(&sim, &nodes[0], &nodes[2]).await;
    talk(&sim, &nodes[0], &nodes[1], "before").await;
    talk(&sim, &nodes[0], &nodes[2], "before").await;
    let (ida, jon, kim) = (key(&nodes[0]), key(&nodes[1]), key(&nodes[2]));

    // Every app message to Jon's routes is stuck before its first hop.
    sim.fault(Fault::SendHangs {
        node: 1,
        hangs: true,
    });
    send(&nodes[0], &jon, "into the hung route").await;
    send(&nodes[0], &kim, "after the hung hint").await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(
            &nodes[2],
            &ida,
            "after the hung hint"
        ))
        .await,
        "a hung hint to Jon did not hold Ida's message to Kim"
    );
    send(&nodes[0], &jon, "behind the hung hint").await;
    sim.fault(Fault::SendHangs {
        node: 1,
        hangs: false,
    });
    for body in ["into the hung route", "behind the hung hint"] {
        assert!(
            eventually(&sim, LIMIT, || holds_message(&nodes[1], &ida, body)).await,
            "Jon reads {body:?}"
        );
    }
    talk(&sim, &nodes[0], &nodes[1], "after the hung send").await;
}
