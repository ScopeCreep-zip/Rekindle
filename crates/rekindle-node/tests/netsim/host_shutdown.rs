//! A Host's exit shuts its transport node down rather than dropping it
//! (plan VU4, A/B R1-16 "transport shutdown with outstanding references —
//! dropping refs=3", C7.27 "database still held at exit"). A dropped node
//! never runs Veilid's `shutdown()`, so its table store and 0.5.8's
//! namespace lock `__veilid_lock` (`table_store/native.rs:113-145`) stay
//! held and no later start in the process can open them.

use crate::harness::start;
use crate::logs;
use crate::pair::{befriend, talk};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_exit_shuts_the_transport_down() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Dee", "Eli"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "bye").await;
    for node in &nodes {
        node.host.shutdown().await.expect("the host stops");
    }
    assert_eq!(
        logs::warned("outstanding references"),
        Vec::<String>::new(),
        "every node was shut down, none dropped"
    );
}
