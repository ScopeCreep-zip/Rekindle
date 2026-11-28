//! The node restart ([`crate::restart`]) against the Veilid lines it cites.

use rekindle_records::port::{DhtBackend, OpenIntent, RecordSchema};

use super::rehydrate_tests::{copies, dht, tick, wipe};
use crate::Netsim;

fn queued(sim: &Netsim, key: &str) -> bool {
    sim.net.inner.lock().nodes[0].rehydrations.contains(key)
}

/// A record the node created is "created this session" until its storage
/// manager terminates (0.5.8 `storage_manager/mod.rs:681-695`): before the
/// restart a `Hold` re-open queues no rehydration and nothing is pushed back
/// to replicas that lost it; after it, the local copy the store kept on disk
/// (`mod.rs:655-672`, `:513-525`) is still there, and the same re-open
/// queues it (`open_record.rs:62-74`) and pushes it again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_record_made_before_a_restart_is_rehydrated_by_a_hold_open_after_it() {
    let (sim, nodes) = Netsim::nodes(2, 36);
    let a = dht(&sim, &nodes[0]);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![7], None).await.unwrap();
    wipe(&sim, &key);
    a.open(&key, None, OpenIntent::Hold).await.unwrap();
    assert!(!queued(&sim, &key), "created this session");
    tick(&sim);
    tick(&sim);
    assert_eq!(
        copies(&sim, &key, 0),
        0,
        "nothing pushed before the restart"
    );
    drop(a);

    sim.terminate_node(0);
    let restarted = sim.start_node(0);
    let a = dht(&sim, &restarted);
    let kept = sim.net.inner.lock().nodes[0]
        .local
        .get(&(key.clone(), 0))
        .map(|v| v.data.clone());
    assert_eq!(kept, Some(vec![7]), "the local store survives the restart");
    assert!(
        sim.net.inner.lock().nodes[0].opened.is_empty(),
        "no record is open after a restart"
    );
    a.open(&key, None, OpenIntent::Hold).await.unwrap();
    assert!(queued(&sim, &key), "made in an earlier session");
    tick(&sim);
    tick(&sim);
    assert!(copies(&sim, &key, 0) > 0, "the kept copy is pushed again");
}
