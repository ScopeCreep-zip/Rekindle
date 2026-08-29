//! The one `VeilidConfig` builder for both node-startup tracks.
//!
//! `RekindleNode::start` (Tauri host) and `TransportNode::start`
//! (daemon/CLI, `rekindle-transport`) used to build near-identical
//! `VeilidConfig` blocks independently, each carrying its own copy of the
//! upstream-workaround comments — and drifting. The tuning knobs are
//! plain data ([`VeilidStartupOptions`], Tier 1); this module owns the
//! only translation of them into `veilid_core::VeilidConfig`.

pub use rekindle_types::config::VeilidStartupOptions;
use veilid_core::VeilidConfig;

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

    vc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_pre_upgrade_behavior() {
        let opts = VeilidStartupOptions::default();
        let vc = build_veilid_config("rekindle", "rekindle", "/tmp/x", &opts);
        assert!(!vc.network.upnp, "UPnP stays off by default");
        assert!(vc.protected_store.always_use_insecure_storage);
        assert!(!vc.protected_store.allow_insecure_fallback);
        assert_eq!(vc.network.rpc.default_route_hop_count, 1);
        assert_eq!(vc.network.dht.max_concurrent_operations, 16);
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
        };
        let vc = build_veilid_config("ns", "q", "/tmp/y", &opts);
        assert!(vc.network.upnp);
        assert!(!vc.protected_store.always_use_insecure_storage);
        assert!(vc.protected_store.allow_insecure_fallback);
        assert_eq!(vc.network.rpc.default_route_hop_count, 3);
        assert_eq!(vc.network.dht.max_concurrent_operations, 32);
    }
}
