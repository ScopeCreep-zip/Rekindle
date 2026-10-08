//! [`SafetyProfile`] lookup keyed on [`MessageClass`].
//!
//! Every message class routes through a Veilid Safe route at the uniform
//! [`ANONYMITY_HOP_FLOOR`] (3-hop Tor-class) — sender hidden behind an
//! ephemeral route id on every path, voice and video included. There is
//! no `Unsafe`/direct path and no per-class hop count: a class only
//! tunes the *latency/ordering* knobs (`stability`, `sequencing`), never
//! its anonymity.
//!
//! - **Voice** — `LowLatency` / `NoPreference`: the lowest-latency
//!   variant *within* the anonymous floor. It does not trade sender
//!   anonymity for speed (a single relay would already link both call
//!   ends; unacceptable on a privsec platform).
//! - **Text + DhtWrite** — `Reliable` / `PreferOrdered`: user-content
//!   that should arrive in order.
//! - **Rpc + DhtRead** — `Reliable` / `NoPreference`: one-shot
//!   request/response, no ordering requirement.

use rekindle_types::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};
use rekindle_types::message::MessageClass;

/// Return the [`SafetyProfile`] appropriate for the given message class.
///
/// All classes share the same `hop_count` ([`ANONYMITY_HOP_FLOOR`]); they
/// differ only in `stability`/`sequencing`. The result is consumed by
/// `rekindle-transport`'s `build_routing_context`, which always builds a
/// `SafetySelection::Safe` route (floor-clamped).
#[must_use]
pub fn profile_for_class(class: MessageClass) -> SafetyProfile {
    let hop_count = ANONYMITY_HOP_FLOOR;
    match class {
        // Voice: lowest-latency variant within the anonymous floor.
        MessageClass::Voice => SafetyProfile {
            hop_count,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
        },
        // Text DMs + DHT writes — ordered, reliable.
        MessageClass::Text | MessageClass::DhtWrite => SafetyProfile {
            hop_count,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
        },
        // RPC invites + DHT reads — reliable, no ordering requirement.
        MessageClass::Rpc | MessageClass::DhtRead => SafetyProfile {
            hop_count,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::NoPreference,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text DMs: 3-hop anonymous, ordered+reliable.
    #[test]
    fn text_class_uses_three_hop_ordered() {
        let p = profile_for_class(MessageClass::Text);
        assert_eq!(p.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(p.stability, StabilityPreference::Reliable);
        assert_eq!(p.sequencing, SequencingPreference::PreferOrdered);
    }

    /// Voice: 3-hop anonymous (same floor as everything else), but the
    /// lowest-latency stability/sequencing within that floor.
    #[test]
    fn voice_class_uses_three_hop_low_latency() {
        let p = profile_for_class(MessageClass::Voice);
        assert_eq!(p.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(p.stability, StabilityPreference::LowLatency);
        assert_eq!(p.sequencing, SequencingPreference::NoPreference);
    }

    /// Rpc: 3-hop anonymous, reliable, no ordering.
    #[test]
    fn rpc_class_uses_three_hop_unordered() {
        let p = profile_for_class(MessageClass::Rpc);
        assert_eq!(p.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(p.stability, StabilityPreference::Reliable);
        assert_eq!(p.sequencing, SequencingPreference::NoPreference);
    }

    /// DhtRead: routing-equivalent to Rpc.
    #[test]
    fn dht_read_matches_rpc() {
        let dr = profile_for_class(MessageClass::DhtRead);
        let r = profile_for_class(MessageClass::Rpc);
        assert_eq!(dr.hop_count, r.hop_count);
        assert_eq!(dr.stability, r.stability);
        assert_eq!(dr.sequencing, r.sequencing);
    }

    /// DhtWrite: routing-equivalent to Text.
    #[test]
    fn dht_write_matches_text() {
        let dw = profile_for_class(MessageClass::DhtWrite);
        let t = profile_for_class(MessageClass::Text);
        assert_eq!(dw.hop_count, t.hop_count);
        assert_eq!(dw.stability, t.stability);
        assert_eq!(dw.sequencing, t.sequencing);
    }

    /// Every class — voice included — routes at the anonymity floor.
    /// Guards against any path silently dropping below 3-hop Safe.
    #[test]
    fn every_class_is_at_the_anonymity_floor() {
        for class in [
            MessageClass::Voice,
            MessageClass::Text,
            MessageClass::Rpc,
            MessageClass::DhtRead,
            MessageClass::DhtWrite,
        ] {
            assert_eq!(
                profile_for_class(class).hop_count,
                ANONYMITY_HOP_FLOOR,
                "{class:?} must route at the 3-hop Tor-class anonymity floor",
            );
        }
    }

    /// Voice + Text share the anonymity floor but differ on the
    /// latency/ordering knobs.
    #[test]
    fn voice_and_text_differ_only_on_perf_knobs() {
        let v = profile_for_class(MessageClass::Voice);
        let t = profile_for_class(MessageClass::Text);
        assert_eq!(v.hop_count, t.hop_count);
        assert_ne!(v.stability, t.stability);
        assert_ne!(v.sequencing, t.sequencing);
    }
}
