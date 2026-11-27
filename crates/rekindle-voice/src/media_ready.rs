//! Media-ready session gate — the WebRTC "ICE→DTLS→SRTP before any
//! RTP" analog for community voice/video over Veilid.
//!
//! Video egress is allowed only once the full bring-up converged:
//! three-way join handshake `Connected`, channel roster non-empty,
//! community MEK cached, local WebCodecs caps reported, and the
//! negotiated `SessionVideoConfig` emitted. Before that, frames would
//! die silently somewhere down the chain (empty roster fan-out, MEK
//! decrypt failure at every receiver, an encoder configured from a
//! codec floor the platform can't encode) — the gate turns each of
//! those into one explicit, named reason.
//!
use rekindle_types::subscription_events::{JoinHandshake, VoiceMediaReady};

/// Everything the gate derives readiness from, for one session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaReadyInputs {
    /// Three-way join handshake stage (`transport::JoinHandshake`).
    pub(crate) handshake: JoinHandshake,
    /// At least one remote peer is in the channel roster. A solo
    /// member correctly never becomes ready — there is nobody to
    /// send to (the reason string distinguishes this from a bug).
    pub roster_non_empty: bool,
    /// The WebView's WebCodecs probe reported real local caps — until
    /// then the negotiated config may name a codec this platform
    /// cannot encode.
    pub local_caps_reported: bool,
    /// `CommunityEvent::VideoSessionConfig` was emitted for this slot,
    /// so the frontend encoder has a negotiated shape to configure from.
    pub session_config_emitted: bool,
}

impl MediaReadyInputs {
    pub(crate) fn snapshot(&self) -> VoiceMediaReady {
        VoiceMediaReady {
            ready: self.ready(),
            reason: self.block_reason().to_string(),
        }
    }

    /// Ready ⇔ handshake fully converged AND all three flags set. There is
    /// no key flag: our media key is our own and exists from the session's
    /// first frame, and peers get it as they add us (plan C7.20).
    #[must_use]
    pub fn ready(&self) -> bool {
        self.handshake == JoinHandshake::Connected
            && self.roster_non_empty
            && self.local_caps_reported
            && self.session_config_emitted
    }

    /// The highest-priority blocker, or `"ready"`. Priority mirrors
    /// bring-up order so the reason always names the NEXT thing the
    /// session is waiting on.
    #[must_use]
    pub fn block_reason(&self) -> &'static str {
        match self.handshake {
            JoinHandshake::Announced => return "handshake-announced",
            JoinHandshake::Seen => return "handshake-seen",
            JoinHandshake::Connected => {}
        }
        if !self.roster_non_empty {
            return "roster-empty";
        }
        if !self.local_caps_reported {
            return "caps-unreported";
        }
        if !self.session_config_emitted {
            return "config-pending";
        }
        "ready"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_HANDSHAKES: [JoinHandshake; 3] = [
        JoinHandshake::Announced,
        JoinHandshake::Seen,
        JoinHandshake::Connected,
    ];

    /// Build inputs from a 3-bit flag mask: bit0 = roster, bit1 = caps,
    /// bit2 = config (7 = all set).
    fn inputs(handshake: JoinHandshake, bits: u8) -> MediaReadyInputs {
        MediaReadyInputs {
            handshake,
            roster_non_empty: bits & 1 != 0,
            local_caps_reported: bits & 2 != 0,
            session_config_emitted: bits & 4 != 0,
        }
    }

    /// Exhaustive truth table: 3 handshake states × 2³ flags = 24
    /// combinations. `ready()` ⇔ Connected ∧ all three flags, and the
    /// reason is "ready" exactly when ready.
    #[test]
    fn ready_iff_connected_and_all_flags() {
        for hs in ALL_HANDSHAKES {
            for bits in 0..8u8 {
                let i = inputs(hs, bits);
                let expect = hs == JoinHandshake::Connected && bits == 7;
                assert_eq!(i.ready(), expect, "inputs: {i:?}");
                assert_eq!(i.block_reason() == "ready", expect, "inputs: {i:?}");
            }
        }
    }

    /// Reason names the FIRST blocker in bring-up order.
    #[test]
    fn block_reason_priority_order() {
        assert_eq!(
            inputs(JoinHandshake::Announced, 7).block_reason(),
            "handshake-announced"
        );
        assert_eq!(
            inputs(JoinHandshake::Seen, 0).block_reason(),
            "handshake-seen"
        );
        assert_eq!(
            inputs(JoinHandshake::Connected, 0).block_reason(),
            "roster-empty"
        );
        assert_eq!(
            inputs(JoinHandshake::Connected, 1).block_reason(),
            "caps-unreported"
        );
        assert_eq!(
            inputs(JoinHandshake::Connected, 1 | 2).block_reason(),
            "config-pending"
        );
        assert_eq!(inputs(JoinHandshake::Connected, 7).block_reason(), "ready");
    }
}
