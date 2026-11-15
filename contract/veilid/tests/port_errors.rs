//! The `VeilidAPIError` -> port error mapping (`dht/backend.rs` `dht_error`,
//! the pool's `to_port`/`set_failed`, `ports.rs` `send_error` and
//! `detach_result`) on the errors veilid-core's mock API actually returns.

use rekindle_protocol::dht::backend::VeilidDht;
use rekindle_protocol::dht::pool::{set_failed, to_port};
use rekindle_protocol::ports::{peer_route_unusable, VeilidPorts};
use rekindle_records::port::{
    DhtBackend, DhtErrorKind, OpenIntent, PeerTarget, PortError, RecordSchema, SetOutcome,
};
use rekindle_veilid_contract::{node_config, profile, start_node, MockNode};
use tokio::sync::watch;
use veilid_core::VeilidAPIError;

/// A well-formed record key no node created (bytes 1..=32).
const UNKNOWN_KEY: &str = "VLD0:AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";

/// A well-formed route id no node allocated (bytes 1..=32).
const UNKNOWN_ROUTE: &str = "VLD0:AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";

fn backend(node: &MockNode) -> VeilidDht {
    let rc = node.api.routing_context().expect("routing context");
    VeilidDht::new(&rc, &profile()).expect("backend")
}

/// Each mock refusal reaches the pool as the kind Veilid named, with
/// Veilid's text, and classifies as the pool reads that kind.
#[tokio::test]
async fn dht_refusals_keep_their_kind_and_classify() {
    let a = start_node("errors-dht-a").await;
    let b = start_node("errors-dht-b").await;
    let dht_a = backend(&a);
    let dht_b = backend(&b);
    let created = dht_a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .expect("create");

    // No node holds it: `KeyNotFound` (`mock/records.rs:236-241`),
    // unreachable to the pool.
    let missing = dht_b
        .open(UNKNOWN_KEY, None, OpenIntent::Check)
        .await
        .expect_err("unknown");
    assert_eq!(missing.kind, DhtErrorKind::KeyNotFound, "{missing}");
    assert!(matches!(to_port(&missing), PortError::Unreachable(_)));

    // Not open on b: `InvalidArgument` "record not open"
    // (`mock/records.rs:115-124`), a DHT error.
    let not_open = dht_b
        .set(&created.key, 0, b"x".to_vec(), created.owner.as_ref())
        .await
        .expect_err("not open on b");
    assert_eq!(not_open.kind, DhtErrorKind::InvalidArgument, "{not_open}");
    assert!(not_open.message.contains("record not open"), "{not_open}");
    assert!(matches!(set_failed(&not_open), Err(PortError::Dht(_))));

    // Open with no writer: `Generic` "value is not writable"
    // (`mock/records.rs:560-562`).
    dht_b
        .open(&created.key, None, OpenIntent::Check)
        .await
        .expect("open");
    let read_only = dht_b
        .set(&created.key, 0, b"x".to_vec(), None)
        .await
        .expect_err("no writer");
    assert_eq!(read_only.kind, DhtErrorKind::Generic, "{read_only}");
    assert!(matches!(to_port(&read_only), PortError::Dht(_)));

    // Detached: a write `TryAgain`s (`mock/records.rs:611-615`), which
    // the pool reads as `Offline`; an open that must reach the network
    // does too (:229-233), unreachable to the pool.
    b.api.detach().await.expect("detach");
    let offline = dht_b
        .set(&created.key, 0, b"x".to_vec(), created.owner.as_ref())
        .await
        .expect_err("offline");
    assert_eq!(offline.kind, DhtErrorKind::TryAgain, "{offline}");
    assert_eq!(set_failed(&offline), Ok(SetOutcome::Offline));
    let unheld = dht_b
        .open(UNKNOWN_KEY, None, OpenIntent::Check)
        .await
        .expect_err("offline open");
    assert_eq!(unheld.kind, DhtErrorKind::TryAgain, "{unheld}");
    assert!(matches!(to_port(&unheld), PortError::Unreachable(_)));

    // Shut down: `NotInitialized`, with Veilid's text.
    b.api.clone().shutdown().await;
    let gone = dht_b
        .open(&created.key, None, OpenIntent::Check)
        .await
        .expect_err("shut down");
    assert_eq!(gone.kind, DhtErrorKind::NotInitialized, "{gone}");
    assert_eq!(gone.message, VeilidAPIError::NotInitialized.to_string());
}

/// Sends through `VeilidPorts`' message port: a live route delivers; the
/// mock's answer for a route it does not know is `InvalidTarget` "unknown
/// route" (`mock/app_messaging.rs:79-93`), which is our side, a `Send`
/// error that keeps the peer's route; a detach of a detached node is
/// success ("Not attached", mock `api.rs:453-460`, real
/// `attachment_manager/mod.rs:527-528`).
#[tokio::test]
async fn sends_and_detach_through_the_ports() {
    let (ready_tx, ready) = watch::channel(true);
    let (ports, _events) = VeilidPorts::start(&node_config("errors-ports"), profile(), ready)
        .await
        .expect("ports");
    let mut peer = start_node("errors-ports-peer").await;
    let blob = peer.api.new_private_route().await.expect("route").blob;

    let target = ports.routes.import(&blob).expect("import");
    ports
        .messages
        .send(&target, &profile(), b"hello".to_vec())
        .await
        .expect("a live route delivers");
    assert!(matches!(
        peer.next_event().await,
        rekindle_records::port::PortEvent::AppMessage { message } if message == b"hello"
    ));

    let unknown = ports
        .messages
        .send(
            &PeerTarget::new(UNKNOWN_ROUTE.into()),
            &profile(),
            b"x".to_vec(),
        )
        .await
        .expect_err("unknown route");
    assert!(matches!(unknown, PortError::Send(_)), "{unknown:?}");

    let lifecycle = ports.lifecycle.as_ref().expect("a started node");
    lifecycle.detach().await.expect("detach");
    lifecycle.detach().await.expect("a detached node detaches");
    lifecycle.shutdown().await;
    drop(ready_tx);
}

/// `RouteUnusable` is only the peer's unresolved route: real veilid-core
/// answers it as `NoConnection` "could not get remote private route"
/// (`rpc_processor/destination.rs:369`, `rpc_processor/error.rs:49-51`).
/// The mock cannot produce it (its unresolved route is `InvalidTarget`,
/// `mock/app_messaging.rs:84-88`), so it is checked on the error itself.
#[test]
fn only_the_peers_unresolved_route_is_unusable() {
    assert!(peer_route_unusable(&VeilidAPIError::no_connection(
        "could not get remote private route"
    )));
    for ours in [
        VeilidAPIError::no_connection("no routing domain"),
        VeilidAPIError::invalid_target("unknown route"),
        VeilidAPIError::timeout(),
    ] {
        assert!(!peer_route_unusable(&ours), "{ours}");
    }
}
