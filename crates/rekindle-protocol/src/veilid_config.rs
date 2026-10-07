//! The one `VeilidConfig` builder for both node-startup tracks.
//!
//! `RekindleNode::start` (Tauri host) and `TransportNode::start`
//! (daemon/CLI, `rekindle-transport`) used to build near-identical
//! `VeilidConfig` blocks independently, each carrying its own copy of the
//! upstream-workaround comments — and drifting. The tuning knobs are
//! plain data ([`VeilidStartupOptions`], Tier 1); this module owns the
//! only translation of them into `veilid_core::VeilidConfig`.

use std::time::Duration;

pub use rekindle_types::config::VeilidStartupOptions;
use veilid_core::VeilidConfig;

/// Veilid's per-call timeout: `max(get_value, set_value, resolve_node,
/// 2 × rpc)` under the internal defaults, which veilid-core resets
/// non-default values to unless built with `footgun-config` (we are not).
/// It is not an upper bound on a DHT call: before consensus a fanout's
/// deadline moves to a full timeout past its last accepting node
/// (`rpc_processor/fanout/fanout_call.rs:642-656`), plus the operation-gate
/// and record-lock waits. There is no cancellation API (veilid issue #516).
pub const LONGEST_VEILID_CALL: Duration = Duration::from_secs(10);

/// How long a session's scope gets to stop at logout, lock or exit before
/// its stragglers are aborted (`evidence/c4-live-findings.md` L1). Only
/// tasks that make no Veilid call directly may be aborted: the DHT work of
/// a session task is record-pool calls, which run on the pool's scope, are
/// released by its drain and are never aborted (an abort mid-commit wedges
/// Veilid's record store; plan C7.6g, C7.6j).
pub const SESSION_STOP_DEADLINE: Duration = Duration::from_secs(12);

/// Build the Veilid startup config from plain-data options.
///
/// - `program_name`: application namespace on the Veilid network.
/// - `qualifier`: passed to `VeilidConfig::new()` (e.g. "rekindle" or
///   "rekindle-server").
/// - `storage_dir`: base directory for Veilid's protected/table/block
///   stores.
pub fn build_veilid_config(
    program_name: &str,
    qualifier: &str,
    storage_dir: &str,
    opts: &VeilidStartupOptions,
) -> VeilidConfig {
    let mut vc = VeilidConfig::new(
        program_name,
        "com", // organization
        qualifier,
        Some(storage_dir), // storage_directory override
        None,              // config_directory (use default)
    );

    // ProtectedStore mode. Defaults to file-backed (insecure) storage:
    // veilid-core 0.5.3's keyring-manager 0.7.1 reached secret-service's
    // BLOCKING zbus API on Linux and panicked inside our Tokio runtime
    // ("cannot start a runtime from within a runtime") before any
    // fallback could run. 0.5.7 resolves keyring-manager 0.8.3, which is
    // UNTESTED against that panic — retest protocol in
    // docs/contributor/veilid-0.5.7-migration-plan.md §2.3. Low stakes:
    // this store only guards Veilid's own node/route secrets (they live
    // in `storage_dir` regardless); user identity keys are in the
    // SQLCipher vault.
    vc.protected_store.allow_insecure_fallback = opts.allow_insecure_fallback;
    vc.protected_store.always_use_insecure_storage = opts.always_use_insecure_storage;

    // UPnP/IGD automatic port mapping. Defaults off; without it a NAT'd
    // node degrades gracefully to inbound relays via VICE. History and
    // the flip protocol: docs/contributor/veilid-0.5.7-gap-audit.md §1.1
    // (the 0.5.3-era panic is gone in 0.5.7, but the
    // restart-on-IGD-failure loop is not, and the original failure was
    // never conclusively diagnosed — flip only under attachment-flap
    // monitoring). Note: 0.5.7 silently disables UPnP whenever
    // `privacy.require_inbound_relay` is set.
    vc.network.upnp = opts.upnp;

    // Inbound private-route hops. Default 1: compiled paths are
    // safety(3)+private(1) = 4 hops, at/above the architecture §8 target.
    // Rationale for 1 vs 3 and the retest protocol:
    // rekindle-types `VeilidStartupOptions::route_hop_count` docs and
    // migration-plan §4.1.
    vc.network.rpc.default_route_hop_count = opts.route_hop_count;

    // Global DHT concurrency ceiling (0.5.4+). Upstream default is 16;
    // the per-subsystem semaphores stay as fairness limits beneath it.
    vc.network.dht.max_concurrent_operations = opts.max_concurrent_dht_operations;

    // Records stored on behalf of other nodes. veilid-server's node value
    // rather than the embedded 128, so the byte cap is what binds and
    // eviction runs through the record index's size accounting
    // (`VeilidStartupOptions::remote_max_records`).
    vc.network.dht.remote_max_records = opts.remote_max_records;

    // App nodes do not serve the DHT (plan C7.10). A node that comes and
    // goes holds values that vanish with it and drag consensus down: upstream
    // wants DHTV counted only once a node's peer info is old enough (veilid
    // #492), and VeilidChat disables it on its nodes until then
    // (`veilid_support/lib/src/config.dart:230-234`). Disabled, the node
    // answers others' get/set/inspect/watch with "dht is not available"
    // (`rpc_get_value.rs:191-198` and siblings) and fanouts skip it; our own
    // DHT calls fan out to DHTV nodes as before, and the keepalive's
    // rehydrate is an outbound set (`rehydrate.rs:50-57`), so mutual aid is
    // unchanged. It also stops this node caching others' records, whose
    // reads a caching node can observe (veilid #319). Not a knob: no app
    // node should serve until #492 lands.
    vc.capabilities.disable = vec![veilid_core::VEILID_CAPABILITY_DHT];

    vc
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(SESSION_STOP_DEADLINE > LONGEST_VEILID_CALL);
    }

    #[test]
    fn defaults_preserve_pre_upgrade_behavior() {
        let opts = VeilidStartupOptions::default();
        let vc = build_veilid_config("rekindle", "rekindle", "/tmp/x", &opts);
        assert!(!vc.network.upnp, "UPnP stays off by default");
        assert!(vc.protected_store.always_use_insecure_storage);
        assert!(!vc.protected_store.allow_insecure_fallback);
        assert_eq!(vc.network.rpc.default_route_hop_count, 1);
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
}
