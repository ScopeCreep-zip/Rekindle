//! A first contact reads one slot (R1-8, plan E2.4 B2): Ann accepts Rae's
//! request and opens the session from Rae's prekey record. The accept
//! inspects the record once and reads its head and **one** one-time slot,
//! drawn among those the inspect shows (Signal's bundle carries one
//! one-time prekey, PQXDH §3.3), not all 100 slots: on a first contact
//! every slot is network-only, so reading them all cost ten waves of gets
//! (21–29 s live) before the PreKeyInit could go.
//!
//! The scenario counts the gets Ann's app sends to the network for Rae's
//! record, and times its reads: the first op on the record (the inspect)
//! to the start of the last get, plus that get's longest answer, is at
//! most two round trips.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use rekindle_netsim::latency::GET;
use tokio::time::Instant;

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request, talk, LIMIT};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_contact_reads_the_head_and_one_slot() {
    let (sim, mut nodes) = start(2, 53).await;
    for (node, name) in nodes.iter_mut().zip(["Rae", "Ann"]) {
        node.unlock_new_identity(name).await;
    }
    request(&sim, &nodes[0], &nodes[1]).await;
    let record = nodes[0]
        .prekey_status()
        .await
        .record_key
        .expect("Rae's prekey record");
    let ops = || (sim.app_gets(1, &record), sim.app_inspects(1, &record));
    let (gets_before, _) = ops();

    // Ann's accept, with each change of her op counts on the record timed.
    let done = Cell::new(false);
    let seen: RefCell<Vec<Instant>> = RefCell::new(Vec::new());
    let accepting = async {
        accept(&nodes[1], &nodes[0]).await;
        done.set(true);
    };
    let sampling = async {
        let mut last = ops();
        while !done.get() {
            let now = ops();
            if now != last {
                seen.borrow_mut().push(Instant::now());
                last = now;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    futures::join!(accepting, sampling);

    let gets = sim.app_gets(1, &record) - gets_before;
    assert!(
        gets <= 3,
        "the accept sent {gets} gets for Rae's prekey record"
    );
    let seen = seen.into_inner();
    let (first, last) = (
        *seen.first().expect("the accept read Rae's record"),
        *seen.last().expect("the accept read Rae's record"),
    );
    let reads = last - first + GET.max;
    assert!(
        reads <= 2 * GET.max,
        "the accept's reads of Rae's record took {reads:?}"
    );

    let ann = key(&nodes[1]);
    assert!(
        eventually(&sim, LIMIT, || is_accepted_friend(&nodes[0], &ann)).await,
        "the session reaches Rae"
    );
    talk(&sim, &nodes[0], &nodes[1], "after one slot").await;
}
