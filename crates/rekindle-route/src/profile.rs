//! The [`SafetyProfile`] of a [`MessageClass`]. Rekindle's one
//! class-to-profile table is [`SafetyConfig`]'s defaults (`rekindle-types`
//! `config`), looked up by [`SafetyConfig::for_class`]; this is that table
//! before user config and the admin hop floor apply, for the senders that
//! hold no `TransportConfig` (the desktop's own Veilid contexts). The
//! transport reads its configured copy through the same lookup.
//!
//! | Class | Stability | Sequencing (Veilid) | Consumers |
//! |---|---|---|---|
//! | Text | Reliable | PreferOrdered | transport `Sender::send_direct`, `AppCaller::call_direct`; desktop `state_helpers` `safe_routing_context_from`, `routes::safe_profile` |
//! | Voice | LowLatency | PreferUnordered (`NoPreference`) | desktop `VeilidVoiceFrameSender` (voice and video frames) |
//! | Rpc | Reliable | EnsureOrdered | transport `AppCaller::call`, `call_community_envelope`, `send_unframed_to_route` |
//! | Dht | Reliable | PreferOrdered | transport node start (`VeilidPorts::start`: the record pool's safety selection) |
//!
//! Every class rides a Safe route at
//! [`ANONYMITY_HOP_FLOOR`](rekindle_types::config::ANONYMITY_HOP_FLOOR); a class tunes
//! only stability and sequencing. Control traffic is ordered and media
//! unordered (plan VU decision 2), Veilid's intended use: `Sequencing`
//! defaults to PreferOrdered (veilid-core `veilid_api/types/safety.rs:12-21`);
//! a routed operation carries the sender's sequencing and every hop filters
//! the next hop by it (`rpc_processor/rpc_route.rs:26-29`, `:87-90`); Veilid
//! adapts it only to the sender's own safety route (`route_compile.rs:106-110`),
//! never to the receiver's private route, whose hops were chosen and proven
//! on its one ordering (`route_allocate.rs:580-588`). An unordered send to an
//! Ordered General route crosses UDP hop pairs nobody proved: in the real app
//! (DL1/DL2, 2026-11-25) such sends were lost while ordered ones arrived.

use rekindle_types::config::{SafetyConfig, SafetyProfile};
use rekindle_types::message::MessageClass;

/// The default [`SafetyProfile`] of `class`: [`SafetyConfig::for_class`]
/// over the default table.
#[must_use]
pub fn profile_for_class(class: MessageClass) -> SafetyProfile {
    SafetyConfig::default().for_class(class).clone()
}

#[cfg(test)]
mod tests {
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
}
