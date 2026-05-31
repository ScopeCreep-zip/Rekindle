//! Phase 9 — `SafetyProfile` lookup keyed on [`MessageClass`].
//!
//! Different message categories demand different routing trade-offs.
//! Voice frames need 0-hop direct routes (every relay adds ~50 ms of
//! latency, which kills audio quality). Text DMs can tolerate the
//! latency in exchange for sender anonymity. DHT operations split:
//! reads need only 1 relay (readers can be less anonymous; sender
//! anonymity less critical), writes need 2 + sender anonymity (don't
//! leak who's publishing what).
//!
//! The mapping below comes from the plan's § Phase 9 table verbatim.
//! `sender_anonymous` distinguishes:
//! - **Text vs DhtWrite** — same hops/stability/sequencing, but Text
//!   typically targets one known peer (anonymity helps relay-side); DhtWrite
//!   is broadcast-shaped (anonymity protects publish patterns).
//! - **Rpc vs DhtRead** — same shape, but Rpc carries application
//!   payloads requiring sender privacy; DhtRead is infrastructure
//!   (less identity-leak risk).
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 9.

use rekindle_types::config::{SafetyProfile, SequencingPreference, StabilityPreference};
use rekindle_types::message::MessageClass;

/// Return the [`SafetyProfile`] appropriate for the given message class.
///
/// The mapping is verbatim from the plan's § Phase 9 table. Five
/// classes collapse to **three** distinct profiles because:
///
/// - **Text + DhtWrite** share routing: both carry user-attributable
///   content; sender anonymity matters; 2-hop safety route + ordered
///   reliable transport. The classes stay distinct at the type level
///   for call-site clarity (DM service vs DHT manager) but Veilid's
///   `SafetySelection::Safe(SafetySpec)` treats them identically.
/// - **Rpc + DhtRead** share routing: 1-hop safety route, reliable,
///   no ordering requirement, sender anonymous. DhtRead is
///   `sender_anonymous: true` (per plan + `threat-model.md` Z12 —
///   DHT lookups leak the social graph if the sender is identifiable;
///   adversary A4 "compromised relay" + A10 "state-level surveillance"
///   both rely on linking lookup traffic to a real node identity).
/// - **Voice** is the only `Unsafe` path: 0-hop direct from the
///   personal private route. Audio participants are mutually known
///   by design (you can't call someone anonymously), and every relay
///   adds ~50 ms latency, which destroys call quality.
///
/// `sender_anonymous` is what distinguishes `SafetySelection::Safe`
/// (anonymous via safety route) from `SafetySelection::Unsafe`
/// (direct from personal route). The mapping is consumed by
/// `rekindle-transport`'s `routing_context_for_profile` to pick the
/// Veilid call variant.
#[must_use]
pub fn profile_for_class(class: MessageClass) -> SafetyProfile {
    match class {
        // Voice is the only non-anonymous path. Latency over privacy
        // because call participants are mutually known.
        MessageClass::Voice => SafetyProfile {
            hop_count: 0,
            stability: StabilityPreference::LowLatency,
            sequencing: SequencingPreference::NoPreference,
            sender_anonymous: false,
        },
        // Text DMs + DHT writes — 2-hop safety route, ordered+reliable.
        // Both carry user-content; sender must be hidden from the
        // destination's incoming relays.
        MessageClass::Text | MessageClass::DhtWrite => SafetyProfile {
            hop_count: 2,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::PreferOrdered,
            sender_anonymous: true,
        },
        // RPC invites + DHT reads — 1-hop safety route, reliable, no
        // ordering. DhtRead uses sender_anonymous=true per the plan
        // because DHT lookup traffic, if attributable to a node
        // identity, leaks the social graph (which peers/communities
        // does this node care about). See docs/security/threat-model.md
        // Z12 and I8-I12.
        MessageClass::Rpc | MessageClass::DhtRead => SafetyProfile {
            hop_count: 1,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::NoPreference,
            sender_anonymous: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text DMs: 2-hop safety route, sender anonymous.
    #[test]
    fn text_class_uses_two_hop_anonymous() {
        let p = profile_for_class(MessageClass::Text);
        assert_eq!(p.hop_count, 2);
        assert_eq!(p.stability, StabilityPreference::Reliable);
        assert_eq!(p.sequencing, SequencingPreference::PreferOrdered);
        assert!(p.sender_anonymous);
    }

    /// Voice: 0-hop direct, sender NOT anonymous (call participants are
    /// mutually known by design).
    #[test]
    fn voice_class_uses_zero_hop_direct_sender() {
        let p = profile_for_class(MessageClass::Voice);
        assert_eq!(p.hop_count, 0);
        assert_eq!(p.stability, StabilityPreference::LowLatency);
        assert_eq!(p.sequencing, SequencingPreference::NoPreference);
        assert!(!p.sender_anonymous);
    }

    /// Rpc: 1-hop safety route, sender anonymous.
    #[test]
    fn rpc_class_uses_one_hop_anonymous() {
        let p = profile_for_class(MessageClass::Rpc);
        assert_eq!(p.hop_count, 1);
        assert_eq!(p.stability, StabilityPreference::Reliable);
        assert_eq!(p.sequencing, SequencingPreference::NoPreference);
        assert!(p.sender_anonymous);
    }

    /// DhtRead: identical to Rpc routing — sender anonymous per plan
    /// (lookups leak the social graph if attributable to a node).
    #[test]
    fn dht_read_uses_one_hop_anonymous_same_as_rpc() {
        let dr = profile_for_class(MessageClass::DhtRead);
        let r = profile_for_class(MessageClass::Rpc);
        assert_eq!(dr.hop_count, 1);
        assert!(dr.sender_anonymous, "DHT reads must hide which peers we query");
        // Plan-stated: routing-equivalent to Rpc.
        assert_eq!(dr.hop_count, r.hop_count);
        assert_eq!(dr.stability, r.stability);
        assert_eq!(dr.sequencing, r.sequencing);
        assert_eq!(dr.sender_anonymous, r.sender_anonymous);
    }

    /// DhtWrite: identical to Text routing — sender anonymous.
    #[test]
    fn dht_write_uses_two_hop_anonymous_same_as_text() {
        let dw = profile_for_class(MessageClass::DhtWrite);
        let t = profile_for_class(MessageClass::Text);
        assert_eq!(dw.hop_count, 2);
        assert!(dw.sender_anonymous);
        // Plan-stated: routing-equivalent to Text.
        assert_eq!(dw.hop_count, t.hop_count);
        assert_eq!(dw.stability, t.stability);
        assert_eq!(dw.sequencing, t.sequencing);
        assert_eq!(dw.sender_anonymous, t.sender_anonymous);
    }

    /// Voice is the ONLY non-anonymous class. Every other class must
    /// have sender_anonymous=true. Guards against accidentally exposing
    /// a user's node identity on any non-voice path.
    #[test]
    fn only_voice_is_non_anonymous() {
        for class in [
            MessageClass::Text,
            MessageClass::Rpc,
            MessageClass::DhtRead,
            MessageClass::DhtWrite,
        ] {
            let p = profile_for_class(class);
            assert!(
                p.sender_anonymous,
                "{class:?} must be sender-anonymous; only Voice is permitted to use Unsafe",
            );
        }
        assert!(!profile_for_class(MessageClass::Voice).sender_anonymous);
    }

    /// Voice + Text are clearly distinct on multiple axes.
    #[test]
    fn voice_and_text_are_distinct() {
        let v = profile_for_class(MessageClass::Voice);
        let t = profile_for_class(MessageClass::Text);
        assert_ne!(v.hop_count, t.hop_count);
        assert_ne!(v.stability, t.stability);
        assert_ne!(v.sender_anonymous, t.sender_anonymous);
    }
}
