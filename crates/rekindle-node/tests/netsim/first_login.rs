//! A fresh identity's first login (plan E2.B): its own records — the
//! prekey head and the first-contact slots, the profile naming them, and
//! the friend list — are on the network within about two set latencies of
//! the unlock, and another node reads the profile then. The live log of
//! 2026-10-21 took ~3 min, each write awaited after the one before.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{latency, Netsim};
use rekindle_types::dht_layout::prekeys::{slot_subkey, HEAD};

use crate::harness::{self, PASSPHRASE};

/// The slots the unlock writes before the profile names the record (the
/// live log's 11 writes: the head and these).
const FIRST_CONTACT_SLOTS: u32 = 10;

/// Every own record's first subkeys hold a value on the network.
fn published(sim: &Netsim, prekeys: &str, profile: &str, friends: &str) -> bool {
    let first = (0..FIRST_CONTACT_SLOTS).map(slot_subkey);
    std::iter::once(HEAD)
        .chain(first)
        .all(|subkey| sim.network_value(prekeys, subkey).is_some())
        && sim.network_value(profile, 0).is_some()
        && sim.network_value(friends, 0).is_some()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E2.B finding F1: serial own-record writes"]
async fn a_fresh_identity_publishes_within_two_set_latencies() {
    let (sim, nodes) = harness::start(1, 41).await;
    let reader = sim.add_node();
    sim.set_ready(reader.index, true);
    let unlocked = tokio::time::Instant::now();
    nodes[0]
        .ok(IpcRequest::IdentityCreate {
            display_name: Some("Fay".into()),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let budget = latency::SET.max * 2;
    let limit = Duration::from_secs(600);
    let profile = loop {
        let shown = nodes[0].ok(IpcRequest::IdentityShow).await;
        let key = |field: &str| shown[field].as_str().unwrap_or_default().to_string();
        let (profile, friends) = (key("profileDhtKey"), key("friendListDhtKey"));
        let prekeys = nodes[0].prekey_status().await.record_key;
        let done = prekeys.as_deref().is_some_and(|prekeys| {
            !profile.is_empty()
                && !friends.is_empty()
                && published(&sim, prekeys, &profile, &friends)
        });
        if done {
            break profile;
        }
        assert!(unlocked.elapsed() < limit, "own records not published");
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let took = unlocked.elapsed();
    assert!(
        took <= budget,
        "own records published in {took:?}, past two set latencies ({budget:?})"
    );
    let pool = reader.ports.records.start_session().expect("a session");
    let lease = pool
        .acquire(&profile, None)
        .await
        .expect("open the profile");
    let read = pool.get(lease, 0, true).await.expect("read the profile");
    assert!(read.is_some(), "a second node reads the profile");
}
