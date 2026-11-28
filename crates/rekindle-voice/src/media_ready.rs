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
#[path = "media_ready_tests.rs"]
mod tests;
