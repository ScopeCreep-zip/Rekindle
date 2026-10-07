//! Transport configuration: the `[network]` section of `config.toml`.
//!
//! [`TransportConfig`] is both the transport's runtime configuration and
//! the user-facing `[network]` schema (snake_case keys, unknown keys
//! rejected), so the file format and the node agree by construction.
//! `rekindled` loads it (`user::UserConfig`) and adds its storage
//! directory; `rekindle config validate` and the TUI read the same types.

pub mod user;

use serde::{Deserialize, Serialize};

/// Veilid Safe-route relay hops applied to **every** path, voice and video included.
///
/// 3 = "Tor-class": a middle relay means no guard+exit collusion of two nodes can
/// link sender to receiver. This is the single anonymity floor — `hop_count` is never
/// lowered per data class; only `stability`/`sequencing` vary. veilid-core also rejects
/// a Safe route with `hop_count == 0`, so the floor is always a valid route.
pub const ANONYMITY_HOP_FLOOR: u8 = 3;

/// How long a session's scope gets to stop at logout, lock or exit before
/// its stragglers are aborted (`evidence/c4-live-findings.md` L1). Only
/// tasks that make no Veilid call directly may be aborted: the DHT work of
/// a session task is record-pool calls, which run on the pool's scope, are
/// released by its drain and are never aborted (an abort mid-commit wedges
/// Veilid's record store; plan C7.6g, C7.6j).
pub const SESSION_STOP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12);

/// Top-level transport configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportConfig {
    /// Base storage directory for Veilid persistent state. Set by the host
    /// from its data root, never read from the config file.
    #[serde(skip)]
    pub storage_dir: String,

    /// Application namespace on the Veilid network.
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Per-data-class safety routing parameters.
    #[serde(default)]
    pub safety: SafetyConfig,

    /// Maximum retry attempts for failed DHT writes.
    #[serde(default = "default_dht_write_retries")]
    pub dht_write_retries: u32,

    /// Cadence in seconds of the attached-but-routeless watchdog.
    /// Routes are event-driven (healed on `RouteChange` death), never
    /// rotated on a timer — this only backstops missed heals.
    #[serde(default = "default_route_watchdog_secs")]
    pub route_watchdog_secs: u64,

    /// TTL in seconds for cached imported routes before re-import.
    #[serde(default = "default_route_cache_ttl_secs")]
    pub route_cache_ttl_secs: u64,

    /// Circuit breaker: consecutive failures before tripping.
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,

    /// Circuit breaker: cooldown period in seconds after tripping.
    #[serde(default = "default_circuit_breaker_cooldown_secs")]
    pub circuit_breaker_cooldown_secs: u64,

    /// Gossip dedup cache capacity (number of entries).
    #[serde(default = "default_dedup_cache_capacity")]
    pub dedup_cache_capacity: usize,

    /// Gossip message TTL (max forwarding hops).
    #[serde(default = "default_gossip_ttl")]
    pub gossip_ttl: u8,

    /// Allow Veilid to fall back to its encrypted file-based protected store
    /// when the OS Secret Service (dbus org.freedesktop.secrets) is unavailable.
    ///
    /// Default: false. Set to true in environments without gnome-keyring or
    /// kwallet (containers, headless servers, CI).
    #[serde(default)]
    pub allow_insecure_protected_store: bool,

    /// Veilid node startup tuning (UPnP, DHT concurrency, route hops,
    /// protected-store mode). Shared by both node-startup tracks.
    #[serde(default)]
    pub veilid: VeilidStartupOptions,
}

/// Veilid node startup tuning knobs — the plain-data half of the shared
/// `build_veilid_config()` builder (`rekindle-protocol`), used by both
/// node-startup tracks (`RekindleNode` in the Tauri host,
/// `TransportNode` in the daemon/CLI). Deliberately contains no
/// `veilid_core` types so it can live at Tier 1.
///
/// Every default preserves pre-0.5.7-upgrade behavior; flipping any of
/// these is gated on the live-measurement protocols in
/// `docs/contributor/veilid-0.5.7-migration-plan.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VeilidStartupOptions {
    /// Enable UPnP/IGD automatic port mapping.
    ///
    /// Default `false`. The 0.5.3-era panic that motivated disabling it is
    /// gone in 0.5.7, but the restart-on-IGD-failure loop is not, and the
    /// original failure was never conclusively diagnosed — see
    /// `docs/contributor/veilid-0.5.7-gap-audit.md` §1.1. Flip only under
    /// attachment-flap monitoring.
    #[serde(default)]
    pub upnp: bool,

    /// Always use file-backed (insecure) storage for Veilid's
    /// ProtectedStore instead of the OS keyring.
    ///
    /// Default `true`: keyring-manager 0.7.1 panicked on Linux
    /// (blocking zbus inside our Tokio runtime). 0.5.7 resolves
    /// keyring-manager 0.8.3, which is UNTESTED against that panic — see
    /// migration-plan §2.3 for the retest protocol. Low stakes either
    /// way: this store only guards Veilid's own node/route secrets; user
    /// identity keys live in the SQLCipher vault.
    #[serde(default = "default_true")]
    pub always_use_insecure_storage: bool,

    /// Allow falling back to insecure ProtectedStore storage when the OS
    /// keyring is unavailable. Only meaningful once
    /// `always_use_insecure_storage` is `false`.
    #[serde(default)]
    pub allow_insecure_fallback: bool,

    /// Hops for inbound private routes
    /// (`network.rpc.default_route_hop_count`).
    ///
    /// Default `1` (the Veilid default). A message to us crosses the
    /// sender's safety route, which every Rekindle send builds explicitly
    /// at [`ANONYMITY_HOP_FLOOR`] (3) through `safety_selection` (never the
    /// default context, whose safety route would also use this value: V19,
    /// fixed in plan C7), then this private route: safety(3) + private(1) =
    /// 4 hops, at/above the architecture §8 3-hop target. 3-hop inbound was tried and reverted pre-0.5.4 because
    /// route round-trip testing exhausted the relay pool; 0.5.4's
    /// "simplified route testing" removes that mechanism, so `3` is worth
    /// re-testing — against the original failure symptoms (allocation
    /// flap, empty presence blobs, voice rosters not forming).
    #[serde(default = "default_route_hop_count")]
    pub route_hop_count: u8,

    /// Ceiling on concurrent DHT network operations in flight
    /// (`network.dht.max_concurrent_operations`, added in veilid 0.5.4).
    ///
    /// Default `16` — upstream's own default. This is the global
    /// backpressure ceiling; the per-subsystem semaphores
    /// (`SCAN_PARALLELISM`, `OPEN_PARALLELISM`, …) remain as fairness
    /// limits underneath it.
    #[serde(default = "default_max_concurrent_dht_operations")]
    pub max_concurrent_dht_operations: u32,

    /// Most DHT records this node stores on behalf of other nodes
    /// (`network.dht.remote_max_records`).
    ///
    /// Default `65536`, veilid-server's value for a node that serves the
    /// network; veilid-core's embedded default is 128. Every Veilid app is
    /// also a storage node (developer book, "Configuring Veilid
    /// Applications"), and record survival depends on the nodes nearest a
    /// key holding it. At 128 the count cap binds long before
    /// `remote_max_storage_space_mb` (256 MB): veilid-core 0.5.7's
    /// `make_room_for_record` only makes room by bytes, so each record
    /// past 128 is evicted by the LRU cache itself and logged as
    /// `RecordIndex(remote): Consistency failure, not enough room made`.
    /// With the count cap out of reach, eviction runs through the byte
    /// accounting as designed (verified against veilid-core 0.5.7's
    /// record-index tests; `evidence/veilid-remote-store-limits.md`).
    ///
    /// While app nodes disable DHTV (plan C7.10, until veilid #492) this
    /// node stores no records for others, so the cap is not reached; it
    /// applies again the moment serving is re-enabled.
    #[serde(default = "default_remote_max_records")]
    pub remote_max_records: u32,
}

impl Default for VeilidStartupOptions {
    fn default() -> Self {
        Self {
            upnp: false,
            always_use_insecure_storage: true,
            allow_insecure_fallback: false,
            route_hop_count: default_route_hop_count(),
            max_concurrent_dht_operations: default_max_concurrent_dht_operations(),
            remote_max_records: default_remote_max_records(),
        }
    }
}

/// Per-data-class safety routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyConfig {
    /// Safety profile for text messages (DM + community gossip).
    #[serde(default = "SafetyProfile::default_text")]
    pub text: SafetyProfile,

    /// Safety profile for voice packets.
    #[serde(default = "SafetyProfile::default_voice")]
    pub voice: SafetyProfile,

    /// Safety profile for DHT record operations.
    #[serde(default = "SafetyProfile::default_dht")]
    pub dht: SafetyProfile,

    /// Safety profile for RPC calls (bootstrap, MEK transfer, sync).
    #[serde(default = "SafetyProfile::default_rpc")]
    pub rpc: SafetyProfile,
}

/// Privacy/performance parameters for a single data class.
///
/// Every class routes through a Veilid **safety route** (sender hidden
/// behind an ephemeral route id) at the uniform [`ANONYMITY_HOP_FLOOR`].
/// Classes differ only in `stability`/`sequencing` (latency/ordering
/// knobs) — never in anonymity. Voice keeps `LowLatency` stability, the
/// lowest-latency variant *within* the 3-hop Safe floor, rather than
/// trading anonymity for speed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyProfile {
    /// Safety-route relay hops. Floor-enforced to [`ANONYMITY_HOP_FLOOR`]
    /// by the transport builder; values below it are clamped up.
    #[serde(default = "default_safety_hop_count")]
    pub hop_count: u8,

    /// Prefer connection reliability or low latency.
    #[serde(default)]
    pub stability: StabilityPreference,

    /// Message ordering preference.
    #[serde(default)]
    pub sequencing: SequencingPreference,
}

/// Stability preference -- maps to Veilid's `Stability` enum internally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilityPreference {
    /// Prefer low latency over reliability (may drop packets).
    LowLatency,
    /// Prefer reliable delivery (default).
    #[default]
    Reliable,
}

/// Sequencing preference -- maps to Veilid's `Sequencing` enum internally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequencingPreference {
    /// No ordering guarantee.
    NoPreference,
    /// Prefer ordered delivery (default).
    #[default]
    PreferOrdered,
    /// Require strict ordering (may fail if not achievable).
    EnsureOrdered,
}

impl SafetyProfile {
    pub fn default_text() -> Self {
        Self {
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_voice() -> Self {
        Self {
            // Voice routes through the same 3-hop Tor-class floor as every
            // other class; LowLatency stability is the lowest-latency
            // variant *within* that anonymous floor (not an Unsafe route).
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
        }
    }

    pub fn default_dht() -> Self {
        Self {
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        }
    }

    pub fn default_rpc() -> Self {
        Self {
            hop_count: ANONYMITY_HOP_FLOOR,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::EnsureOrdered,
        }
    }
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            text: SafetyProfile::default_text(),
            voice: SafetyProfile::default_voice(),
            dht: SafetyProfile::default_dht(),
            rpc: SafetyProfile::default_rpc(),
        }
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            storage_dir: "~/.rekindle".into(),
            namespace: default_namespace(),
            safety: SafetyConfig::default(),
            dht_write_retries: default_dht_write_retries(),
            route_watchdog_secs: default_route_watchdog_secs(),
            route_cache_ttl_secs: default_route_cache_ttl_secs(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_cooldown_secs: default_circuit_breaker_cooldown_secs(),
            dedup_cache_capacity: default_dedup_cache_capacity(),
            gossip_ttl: default_gossip_ttl(),
            allow_insecure_protected_store: false,
            veilid: VeilidStartupOptions::default(),
        }
    }
}

fn default_safety_hop_count() -> u8 {
    ANONYMITY_HOP_FLOOR
}
fn default_namespace() -> String {
    "rekindle".into()
}
fn default_true() -> bool {
    true
}
fn default_route_hop_count() -> u8 {
    1
}
fn default_remote_max_records() -> u32 {
    65_536
}

fn default_max_concurrent_dht_operations() -> u32 {
    16
}
fn default_dht_write_retries() -> u32 {
    3
}
fn default_route_watchdog_secs() -> u64 {
    30
}
fn default_route_cache_ttl_secs() -> u64 {
    90
}
fn default_circuit_breaker_threshold() -> u32 {
    3
}
fn default_circuit_breaker_cooldown_secs() -> u64 {
    45
}
fn default_dedup_cache_capacity() -> usize {
    2048
}
fn default_gossip_ttl() -> u8 {
    5
}

/// Highest safety-route hop count accepted: Veilid's own
/// `max_route_hop_count` default.
pub const MAX_ROUTE_HOP_COUNT: u8 = 4;

impl TransportConfig {
    /// Check the constraints serde cannot express. A value below the
    /// anonymity floor is refused rather than clamped, so the file says
    /// what the node does.
    ///
    /// # Errors
    /// The first violated constraint, naming its `network.` key.
    pub fn validate(&self) -> Result<(), user::ConfigError> {
        use user::ConfigError as E;
        if self.namespace.is_empty()
            || self.namespace.len() > 64
            || !self
                .namespace
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(E::new(
                "network.namespace",
                "1-64 characters from [A-Za-z0-9_-]",
            ));
        }
        if !(1..=10).contains(&self.gossip_ttl) {
            return Err(E::new("network.gossip_ttl", "1-10"));
        }
        for (key, value) in [
            (
                "network.circuit_breaker_threshold",
                u64::from(self.circuit_breaker_threshold),
            ),
            ("network.route_watchdog_secs", self.route_watchdog_secs),
            ("network.route_cache_ttl_secs", self.route_cache_ttl_secs),
            (
                "network.dedup_cache_capacity",
                self.dedup_cache_capacity as u64,
            ),
            (
                "network.veilid.remote_max_records",
                u64::from(self.veilid.remote_max_records),
            ),
        ] {
            if value == 0 {
                return Err(E::new(key, "greater than 0"));
            }
        }
        for (class, profile) in [
            ("text", &self.safety.text),
            ("voice", &self.safety.voice),
            ("dht", &self.safety.dht),
            ("rpc", &self.safety.rpc),
        ] {
            if !(ANONYMITY_HOP_FLOOR..=MAX_ROUTE_HOP_COUNT).contains(&profile.hop_count) {
                return Err(E::new(
                    format!("network.safety.{class}.hop_count"),
                    format!("{ANONYMITY_HOP_FLOOR}-{MAX_ROUTE_HOP_COUNT} (the anonymity floor is {ANONYMITY_HOP_FLOOR})"),
                ));
            }
        }
        if !(1..=MAX_ROUTE_HOP_COUNT).contains(&self.veilid.route_hop_count) {
            return Err(E::new(
                "network.veilid.route_hop_count",
                format!("1-{MAX_ROUTE_HOP_COUNT}"),
            ));
        }
        if self.veilid.max_concurrent_dht_operations == 0 {
            return Err(E::new(
                "network.veilid.max_concurrent_dht_operations",
                "greater than 0",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_section_round_trips_as_toml() {
        let cfg = TransportConfig::default();
        let text = toml::to_string(&cfg).unwrap();
        assert!(
            !text.contains("storage_dir"),
            "the host sets the storage dir"
        );
        let parsed: TransportConfig = toml::from_str(&text).unwrap();
        assert_eq!(parsed.namespace, "rekindle");
        assert_eq!(parsed.gossip_ttl, default_gossip_ttl());
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn network_keys_are_snake_case_and_strict() {
        let parsed: TransportConfig = toml::from_str(
            "gossip_ttl = 7\n[safety.voice]\nstability = \"low_latency\"\nsequencing = \"no_preference\"\n",
        )
        .unwrap();
        assert_eq!(parsed.gossip_ttl, 7);
        assert_eq!(parsed.safety.voice.hop_count, ANONYMITY_HOP_FLOOR);
        assert!(toml::from_str::<TransportConfig>("gossipTtl = 1").is_err());
        assert!(toml::from_str::<TransportConfig>("storage_dir = \"/x\"").is_err());
        assert!(toml::from_str::<TransportConfig>("[safety.text]\nhops = 3").is_err());
    }

    #[test]
    fn validation_refuses_hops_below_the_floor() {
        let mut cfg = TransportConfig::default();
        cfg.safety.dht.hop_count = ANONYMITY_HOP_FLOOR - 1;
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.key(), "network.safety.dht.hop_count");
        cfg.safety.dht.hop_count = MAX_ROUTE_HOP_COUNT + 1;
        assert!(cfg.validate().is_err());
        let mut cfg = TransportConfig {
            namespace: "has space".into(),
            ..TransportConfig::default()
        };
        assert!(cfg.validate().is_err());
        cfg.namespace = "ok-name_1".into();
        cfg.gossip_ttl = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn safety_profile_defaults() {
        let text = SafetyProfile::default_text();
        assert_eq!(text.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(text.stability, StabilityPreference::Reliable);

        // Voice rides the same anonymity floor as text; only its
        // stability/sequencing differ (lowest-latency *within* the floor).
        let voice = SafetyProfile::default_voice();
        assert_eq!(voice.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(voice.stability, StabilityPreference::LowLatency);
        assert_eq!(voice.sequencing, SequencingPreference::NoPreference);
    }

    #[test]
    fn every_class_default_is_at_or_above_the_floor() {
        for p in [
            SafetyProfile::default_text(),
            SafetyProfile::default_voice(),
            SafetyProfile::default_dht(),
            SafetyProfile::default_rpc(),
        ] {
            assert!(
                p.hop_count >= ANONYMITY_HOP_FLOOR,
                "no data class may route below the anonymity floor",
            );
        }
    }
}
