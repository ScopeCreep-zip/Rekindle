//! `rekindle-protocol`'s `DhtBackend` impl (`dht/backend.rs`) and the pool's
//! reading of its set results, over veilid-core's mock API (plan VU
//! decisions 1 and 4, VU2 item 15).

use std::sync::Arc;

use rekindle_protocol::dht::backend::{open_options, VeilidDht};
use rekindle_protocol::dht::pool::RecordCloser;
use rekindle_protocol::ports::VeilidRecords;
use rekindle_records::port::{
    DhtBackend, DhtErrorKind, OpenIntent, RecordPort, RecordSchema, RecordStore, SetOutcome,
};
use rekindle_veilid_contract::{profile, start_node};
use tokio::sync::watch;
use veilid_core::VeilidAPI;

/// A record session over `api`, the production path (`ports.rs`
/// `VeilidRecords::start_session`). The sender keeps `ready` true.
fn session(api: &VeilidAPI) -> (Arc<dyn RecordPort>, watch::Sender<bool>) {
    let (ready_tx, ready) = watch::channel(true);
    let records = VeilidRecords::new(api.clone(), profile(), ready, RecordCloser::new());
    (records.start_session().expect("a record session"), ready_tx)
}

/// A well-formed record key no node created (bytes 1..=32).
const UNKNOWN_KEY: &str = "VLD0:AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";

/// Decision 1: `Hold` asks a re-open to rehydrate, `Check` never does.
/// Checked on the options the backend opens with, not through an open:
/// the mock opens from its shared DHT and ignores `rehydrate`
/// (`veilid_api/mock/records.rs:209-220`, the `_options` argument), so no
/// open can show the difference.
#[test]
fn hold_rehydrates_and_check_does_not() {
    assert_eq!(open_options(OpenIntent::Hold).rehydrate, Some(true));
    assert_eq!(open_options(OpenIntent::Check).rehydrate, Some(false));
}

/// Both intents open a stored record through the backend; a key no node
/// holds is Veilid's `KeyNotFound` with Veilid's text
/// (`mock/records.rs:236-241`).
#[tokio::test]
async fn opens_by_intent_and_reports_an_unknown_key() {
    let a = start_node("backend-open-a").await;
    let b = start_node("backend-open-b").await;
    let (pool, _ready) = session(&a.api);
    let (_, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .expect("create");

    let rc = b.api.routing_context().expect("routing context");
    let backend = VeilidDht::new(&rc, &profile()).expect("backend");
    for intent in [OpenIntent::Check, OpenIntent::Hold] {
        let opened = backend.open(&key, None, intent).await.expect("open");
        assert_eq!(opened.key, key);
        assert_eq!(opened.subkey_count, 2);
        assert!(opened.owner.is_none(), "no writer, no owner secret");
    }

    let err = backend
        .open(UNKNOWN_KEY, None, OpenIntent::Check)
        .await
        .expect_err("no node holds it");
    assert_eq!(err.kind, DhtErrorKind::KeyNotFound, "{err}");
}

/// Decision 4 and VU2 item 15, the pool's reading of the backend's set
/// result: `None` is `Landed` (or `Unchanged` for an identical rewrite);
/// `Some(v)` with our data and writer is ours settled (`Landed`); any
/// other `Some(v)` is `Superseded(v)`; `TryAgain` is `Offline`.
#[tokio::test]
async fn set_results_map_per_decision_4() {
    let a = start_node("backend-set-a").await;
    let b = start_node("backend-set-b").await;
    let (pool_a, _ready_a) = session(&a.api);
    let (pool_b, _ready_b) = session(&b.api);
    let (lease_a, key, owner) = pool_a
        .create(&RecordSchema::Dflt { owner_subkeys: 4 }, None)
        .await
        .expect("create");

    // `Ok(None)`: landed, then an identical rewrite keeps its seq
    // (`mock/records.rs:592-596`, `accepts` :46-55).
    let landed = pool_a.set(lease_a, 0, b"one".to_vec(), None).await;
    assert_eq!(landed, Ok(SetOutcome::Landed));
    let again = pool_a.set(lease_a, 0, b"one".to_vec(), None).await;
    assert_eq!(again, Ok(SetOutcome::Unchanged));

    // Seed subkeys 1 (one write) and 2 (two writes, so seq 1) from a.
    for (subkey, data) in [(1, &b"a-1"[..]), (2, b"old"), (2, b"ours")] {
        let out = pool_a.set(lease_a, subkey, data.to_vec(), None).await;
        assert_eq!(out, Ok(SetOutcome::Landed), "subkey {subkey}");
    }

    // b holds the owner key with no local copy, so its writes sign seq 0
    // and lose to what the network holds (`mock/records.rs:592-606`,
    // :641-675 returns the kept value).
    let lease_b = pool_b
        .acquire(&key, Some(&owner), OpenIntent::Hold)
        .await
        .expect("acquire");

    // A different value at the kept seq: `Superseded` with that value.
    match pool_b.set(lease_b, 1, b"b-1".to_vec(), None).await {
        Ok(SetOutcome::Superseded(kept)) => assert_eq!(kept.data, b"a-1"),
        other => panic!("expected Superseded, got {other:?}"),
    }

    // The kept value is ours (same data, same writer, seq 1): ours
    // settled, and the local copy moved, so `Landed`.
    let ours = pool_b.set(lease_b, 2, b"ours".to_vec(), None).await;
    assert_eq!(ours, Ok(SetOutcome::Landed));

    // Detached, `AllowOffline(false)` answers `TryAgain` and stores
    // nothing (`mock/records.rs:611-615`): `Offline`.
    a.api.detach().await.expect("detach");
    let offline = pool_a.set(lease_a, 3, b"late".to_vec(), None).await;
    assert_eq!(offline, Ok(SetOutcome::Offline));
}
