use super::*;
use rekindle_types::config::{SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR};

const CLASSES: [MessageClass; 4] = [
    MessageClass::Text,
    MessageClass::Voice,
    MessageClass::Rpc,
    MessageClass::Dht,
];

/// 1:1 control traffic is reliable and ordered (plan VU decision 2).
#[test]
fn control_traffic_is_reliable_and_ordered() {
    let p = profile_for_class(MessageClass::Text);
    assert_eq!(p.stability, StabilityPreference::Reliable);
    assert_eq!(p.sequencing, SequencingPreference::PreferOrdered);
}

/// Media frames keep UDP: LowLatency and unordered (`NoPreference`,
/// Veilid's PreferUnordered), unlike control traffic.
#[test]
fn media_frames_stay_unordered() {
    let v = profile_for_class(MessageClass::Voice);
    assert_eq!(v.stability, StabilityPreference::LowLatency);
    assert_eq!(v.sequencing, SequencingPreference::NoPreference);
    assert_ne!(
        v.sequencing,
        profile_for_class(MessageClass::Text).sequencing
    );
}

/// RPCs require ordering; DHT operations prefer it.
#[test]
fn rpc_and_dht_rows_are_ordered() {
    let rpc = profile_for_class(MessageClass::Rpc);
    assert_eq!(rpc.stability, StabilityPreference::Reliable);
    assert_eq!(rpc.sequencing, SequencingPreference::EnsureOrdered);
    let dht = profile_for_class(MessageClass::Dht);
    assert_eq!(dht.stability, StabilityPreference::Reliable);
    assert_eq!(dht.sequencing, SequencingPreference::PreferOrdered);
}

/// Every class routes at the anonymity floor, voice included.
#[test]
fn every_class_is_at_the_anonymity_floor() {
    for class in CLASSES {
        assert_eq!(
            profile_for_class(class).hop_count,
            ANONYMITY_HOP_FLOOR,
            "{class:?} must route at the 3-hop Tor-class anonymity floor",
        );
    }
}
