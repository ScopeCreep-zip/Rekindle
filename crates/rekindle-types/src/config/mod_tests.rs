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
