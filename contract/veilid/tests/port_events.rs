//! `VeilidUpdate` -> `PortEvent` (`ports.rs` `to_port_event`), checked by
//! outcome for every variant: which map and to what, which are dropped.
//! The variants the mock emits are driven through it; the rest are built
//! as veilid-core's own values, since the mock compiles out the subsystems
//! that emit them (`core_context.rs:142-143`: under `mock-api` the registry
//! holds only Crypto; `Network` comes from `network_manager/stats.rs:195`,
//! `RouteChange` from `routing_table/route_spec_store/mod.rs:245`, `Log`
//! from the `ApiTracingLayer` an application installs,
//! `logging/api_tracing_layer.rs:16`).

use rekindle_protocol::dht::backend::VeilidDht;
use rekindle_protocol::ports::{to_port_event, VeilidPorts};
use rekindle_records::port::{DhtBackend, OpenIntent, PortEvent, RecordSchema};
use rekindle_veilid_contract::{node_config, profile, start_node, EVENT_WAIT};
use tokio::sync::watch;
use veilid_core::{
    OperationId, Target, VeilidLog, VeilidLogLevel, VeilidRouteChange, VeilidStateNetwork,
    VeilidUpdate,
};

/// `Attachment` maps with the attached flag; `Shutdown` maps; both through
/// the channel `VeilidPorts::start` installs. The mock emits an attachment
/// on attach and detach (`api.rs:409-420`, `:453-461`) and `Shutdown` last
/// (`core_context.rs:228-229`).
#[tokio::test]
async fn attachment_and_shutdown_map_through_the_ports() {
    let (_ready_tx, ready) = watch::channel(true);
    let (ports, mut events) = VeilidPorts::start(&node_config("events-ports"), profile(), ready)
        .await
        .expect("ports");
    let mut next = async || {
        tokio::time::timeout(EVENT_WAIT, events.recv())
            .await
            .expect("an event within the wait")
            .expect("the channel is open")
    };
    assert!(matches!(
        next().await,
        PortEvent::Attachment {
            is_attached: true,
            ..
        }
    ));
    let lifecycle = ports.lifecycle.clone().expect("a started node");
    lifecycle.detach().await.expect("detach");
    match next().await {
        PortEvent::Attachment {
            state,
            is_attached,
            public_internet_ready,
            ..
        } => {
            assert_eq!(state, veilid_core::AttachmentState::Detached.to_string());
            assert!(!is_attached);
            assert!(!public_internet_ready);
        }
        other => panic!("expected Attachment, got {other:?}"),
    }
    lifecycle.shutdown().await;
    assert!(matches!(next().await, PortEvent::Shutdown));
}

/// `AppMessage` and `AppCall` map, and the call id they carry answers the
/// call (`mock/app_messaging.rs:100-150`).
#[tokio::test]
async fn app_message_and_call_map() {
    let a = start_node("events-app-a").await;
    let mut b = start_node("events-app-b").await;
    let blob = b.api.new_private_route().await.expect("route").blob;
    let route = a.api.import_remote_private_route(blob).expect("import");
    let rc = a.api.routing_context().expect("routing context");

    rc.app_message(Target::RouteId(route.clone()), b"note".to_vec())
        .await
        .expect("message");
    assert!(matches!(
        b.next_event().await,
        PortEvent::AppMessage { message } if message == b"note"
    ));

    let call = tokio::spawn(async move {
        rc.app_call(Target::RouteId(route), b"ping".to_vec())
            .await
            .expect("call answered")
    });
    let PortEvent::AppCall { call_id, message } = b.next_event().await else {
        panic!("expected AppCall");
    };
    assert_eq!(message, b"ping");
    b.api
        .app_call_reply(OperationId::new(call_id.0), b"pong".to_vec())
        .await
        .expect("reply");
    assert_eq!(call.await.expect("call task"), b"pong");
}

/// `ValueChange` maps with the record key and subkeys a watch reported
/// (`mock/watches.rs:95-115`).
#[tokio::test]
async fn value_change_maps() {
    let a = start_node("events-value-a").await;
    let mut b = start_node("events-value-b").await;
    let dht_a = VeilidDht::new(&a.api.routing_context().expect("rc"), &profile()).expect("dht");
    let dht_b = VeilidDht::new(&b.api.routing_context().expect("rc"), &profile()).expect("dht");
    let created = dht_a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .expect("create");
    dht_b
        .open(&created.key, None, OpenIntent::Hold)
        .await
        .expect("open");
    assert!(dht_b
        .watch(&created.key, Some(&[1]), None, None)
        .await
        .expect("watch"));

    dht_a
        .set(&created.key, 1, b"v".to_vec(), created.owner.as_ref())
        .await
        .expect("set");
    let PortEvent::ValueChange(change) = b.next_event().await else {
        panic!("expected ValueChange");
    };
    assert_eq!(change.key, created.key);
    assert_eq!(change.subkeys, vec![1]);
}

/// `Config` is dropped: the mock emits it on a config change of a
/// detached node (`api.rs:492-521`).
#[tokio::test]
async fn config_is_dropped() {
    let mut a = start_node("events-config").await;
    a.api.detach().await.expect("detach");
    assert!(matches!(
        a.next_outcome().await,
        Some(PortEvent::Attachment { .. })
    ));
    let config = (*a.api.config().expect("config")).clone();
    a.api.change_config(config).expect("change config");
    assert!(a.next_outcome().await.is_none());
}

/// The variants the mock never emits, as veilid-core builds them:
/// `RouteChange` maps both route lists by id; `Network` and `Log` are
/// dropped.
#[tokio::test]
async fn unemitted_variants_by_outcome() {
    let a = start_node("events-unemitted").await;
    let dead = a.api.new_private_route().await.expect("route").route_id;
    let dead_remote = a.api.new_private_route().await.expect("route").route_id;
    let change = VeilidUpdate::RouteChange(Box::new(VeilidRouteChange {
        dead_routes: vec![dead.clone()],
        dead_remote_routes: vec![dead_remote.clone()],
    }));
    match to_port_event(change) {
        Some(PortEvent::RouteChange {
            dead: mapped,
            dead_remote: mapped_remote,
        }) => {
            assert_eq!(mapped, vec![dead.to_string()]);
            assert_eq!(mapped_remote, vec![dead_remote.to_string()]);
        }
        other => panic!("expected RouteChange, got {other:?}"),
    }

    let network = VeilidUpdate::Network(Box::<VeilidStateNetwork>::default());
    assert!(to_port_event(network).is_none());
    let log = VeilidUpdate::Log(Box::new(VeilidLog {
        log_level: VeilidLogLevel::Info,
        message: "m".into(),
        backtrace: None,
    }));
    assert!(to_port_event(log).is_none());
}
