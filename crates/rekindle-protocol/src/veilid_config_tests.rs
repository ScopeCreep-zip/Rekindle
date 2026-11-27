use super::*;

/// The outbox's DM retry floor counts one Veilid call (finding 49).
#[test]
fn the_call_timeout_is_veilids_rpc_timeout() {
    let net = veilid_core::VeilidConfigInternal::default().network;
    assert_eq!(
        u64::from(net.rpc.timeout_ms),
        rekindle_types::message::CALL_TIMEOUT_MS
    );
}

#[test]
fn longest_call_matches_veilid_defaults() {
    let net = veilid_core::VeilidConfigInternal::default().network;
    let longest_ms = [
        net.dht.get_value_timeout_ms,
        net.dht.set_value_timeout_ms,
        net.dht.resolve_node_timeout_ms,
        2 * net.rpc.timeout_ms,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    assert_eq!(
        LONGEST_VEILID_CALL,
        Duration::from_millis(u64::from(longest_ms))
    );
    assert!(rekindle_types::config::SESSION_STOP_DEADLINE > LONGEST_VEILID_CALL);
}

#[test]
fn defaults_preserve_pre_upgrade_behavior() {
    let opts = VeilidStartupOptions::default();
    let vc = build_veilid_config("rekindle", "rekindle", "/tmp/x", &opts);
    assert!(!vc.network.upnp, "UPnP stays off by default");
    assert!(
        vc.attachment.app_lifecycle.prefer_foreground,
        "a hidden app keeps its foreground guard while attached"
    );
    assert!(
        !vc.network.routing_table.serve_direct_bootstrap,
        "an app node does not serve its peer set"
    );
    assert!(vc.protected_store.always_use_insecure_storage);
    assert!(!vc.protected_store.allow_insecure_fallback);
    assert_eq!(
        vc.network.rpc.default_route_hop_count,
        rekindle_types::config::ANONYMITY_HOP_FLOOR,
        "the pool and our inbound routes take the send hop count (plan RV1)"
    );
    assert_eq!(vc.network.dht.max_concurrent_operations, 16);
    assert_eq!(vc.network.dht.remote_max_records, 65_536);
    assert_eq!(
        vc.capabilities.disable,
        vec![veilid_core::VEILID_CAPABILITY_DHT],
        "app nodes do not serve the DHT"
    );
    assert_eq!(vc.program_name, "rekindle");
}

#[test]
fn options_flow_through() {
    let opts = VeilidStartupOptions {
        upnp: true,
        always_use_insecure_storage: false,
        allow_insecure_fallback: true,
        route_hop_count: 3,
        max_concurrent_dht_operations: 32,
        remote_max_records: 1_000,
    };
    let vc = build_veilid_config("ns", "q", "/tmp/y", &opts);
    assert!(vc.network.upnp);
    assert!(!vc.protected_store.always_use_insecure_storage);
    assert!(vc.protected_store.allow_insecure_fallback);
    assert_eq!(vc.network.rpc.default_route_hop_count, 3);
    assert_eq!(vc.network.dht.max_concurrent_operations, 32);
    assert_eq!(vc.network.dht.remote_max_records, 1_000);
}
