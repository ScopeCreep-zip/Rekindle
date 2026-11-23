//! An `app_call` that never resolves is ended by our own deadline (live
//! finding 78): Veilid bounds only the wait for an answer, so a call stuck
//! before it (a dead route's assembly, a first hop that never takes the
//! envelope) left the receipted handshake row, and the stream behind it,
//! waiting for good. The transport's port bound
//! (`rekindle_transport::broadcast::deadline`) makes it a timeout, and the
//! row is attempted again until it lands.

use std::time::Duration;

use rekindle_netsim::{Fault, Writes};
use rekindle_transport::broadcast::deadline::app_call_deadline;
use rekindle_types::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_call_that_never_resolves_times_out_and_the_row_lands() {
    let (sim, mut nodes) = start(2, 78).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    let ann = key(&nodes[1]);
    request(&sim, &nodes[0], &nodes[1]).await;
    // Every call to Rae's routes hangs while Ann accepts: her receipted
    // FriendAccept is neither answered nor timed out by the network.
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: true,
    });
    // The accept's second path (plan RV2) is held back too, so the accept
    // depends on the call under test: Ann's write of it into Rae's reply
    // inbox lands below consensus until the calls heal.
    sim.fault(Fault::WriteFails {
        node: 1,
        of: Writes::Subkey {
            subkey: rekindle_types::dht_layout::request_inbox::REQUEST,
            record: None,
        },
    });
    accept(&nodes[1], &nodes[0]).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(
        !is_accepted_friend(&nodes[0], &ann).await,
        "no hung call reached Rae"
    );
    sim.fault(Fault::CallHangs {
        node: 0,
        hangs: false,
    });
    sim.fault(Fault::Heal);
    // The attempt in flight ends at our deadline, and the next one lands.
    let bound = app_call_deadline(&SafetyProfile {
        hop_count: ANONYMITY_HOP_FLOOR,
        stability: StabilityPreference::default(),
        sequencing: SequencingPreference::default(),
    });
    assert!(
        eventually(&sim, LIMIT * 20 + bound, || is_accepted_friend(
            &nodes[0], &ann
        ))
        .await,
        "the hung call timed out and the row was attempted again"
    );
    talk(&sim, &nodes[0], &nodes[1], "after the hung call").await;
}
