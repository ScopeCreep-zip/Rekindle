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
//! Pure logic: no I/O, no time, no events. The src-tauri runtime
//! (`services/community/media_ready_runtime.rs`) owns the lock and the
//! `CommunityEvent::VoiceMediaReady` emission; this module only decides.

use std::collections::HashMap;

use crate::transport::JoinHandshake;

/// Everything the gate derives readiness from, per (community, channel).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaReadyInputs {
    /// Three-way join handshake stage (`transport::JoinHandshake`).
    pub handshake: JoinHandshake,
    /// At least one remote peer is in the channel roster. A solo
    /// member correctly never becomes ready — there is nobody to
    /// send to (the reason string distinguishes this from a bug).
    pub roster_non_empty: bool,
    /// The community MEK is cached locally (frames could be encrypted
    /// AND peers hold a generation that can decrypt ours).
    pub mek_present: bool,
    /// The WebView's WebCodecs probe reported real local caps — until
    /// then the negotiated config may name a codec this platform
    /// cannot encode.
    pub local_caps_reported: bool,
    /// `CommunityEvent::VideoSessionConfig` was emitted for this slot,
    /// so the frontend encoder has a negotiated shape to configure from.
    pub session_config_emitted: bool,
}

impl MediaReadyInputs {
    /// Ready ⇔ handshake fully converged AND all four flags set.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.handshake == JoinHandshake::Connected
            && self.roster_non_empty
            && self.mek_present
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
        if !self.mek_present {
            return "mek-missing";
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

/// A (ready, reason) change the runtime must fan out to the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaReadyTransition {
    pub ready: bool,
    pub reason: String,
}

/// Per-(community, channel) input slots + emission dedup. The runtime
/// holds exactly one of these behind a mutex on `AppState`.
#[derive(Debug, Default)]
pub struct MediaReadyTracker {
    slots: HashMap<(String, String), MediaReadyInputs>,
    last_emitted: HashMap<(String, String), (bool, String)>,
}

impl MediaReadyTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutate one slot's inputs (creating it at defaults first) and
    /// report whether the derived (ready, reason) pair CHANGED — the
    /// runtime emits exactly when this returns `Some`.
    pub fn update(
        &mut self,
        community_id: &str,
        channel_id: &str,
        f: impl FnOnce(&mut MediaReadyInputs),
    ) -> Option<MediaReadyTransition> {
        let key = (community_id.to_string(), channel_id.to_string());
        let inputs = self.slots.entry(key.clone()).or_default();
        f(inputs);
        let now = (inputs.ready(), inputs.block_reason().to_string());
        if self.last_emitted.get(&key) == Some(&now) {
            return None;
        }
        self.last_emitted.insert(key, now.clone());
        Some(MediaReadyTransition {
            ready: now.0,
            reason: now.1,
        })
    }

    /// Drop a slot on leave/teardown. Returns a `ready=false`
    /// transition when the slot existed and hadn't already emitted
    /// not-ready — the frontend must re-disable its controls.
    pub fn clear(&mut self, community_id: &str, channel_id: &str) -> Option<MediaReadyTransition> {
        let key = (community_id.to_string(), channel_id.to_string());
        let existed = self.slots.remove(&key).is_some();
        let was = self.last_emitted.remove(&key);
        if existed && was.is_none_or(|(ready, _)| ready) {
            return Some(MediaReadyTransition {
                ready: false,
                reason: "left-channel".to_string(),
            });
        }
        None
    }

    #[must_use]
    pub fn is_ready(&self, community_id: &str, channel_id: &str) -> bool {
        self.slots
            .get(&(community_id.to_string(), channel_id.to_string()))
            .is_some_and(MediaReadyInputs::ready)
    }

    /// Current (ready, reason) for a slot; `(false, "not-in-voice")`
    /// when no session was ever seeded.
    #[must_use]
    pub fn state(&self, community_id: &str, channel_id: &str) -> (bool, String) {
        self.slots
            .get(&(community_id.to_string(), channel_id.to_string()))
            .map_or_else(
                || (false, "not-in-voice".to_string()),
                |i| (i.ready(), i.block_reason().to_string()),
            )
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

    /// Build inputs from a 4-bit flag mask: bit0 = roster, bit1 = mek,
    /// bit2 = caps, bit3 = config (15 = all set).
    fn inputs(handshake: JoinHandshake, bits: u8) -> MediaReadyInputs {
        MediaReadyInputs {
            handshake,
            roster_non_empty: bits & 1 != 0,
            mek_present: bits & 2 != 0,
            local_caps_reported: bits & 4 != 0,
            session_config_emitted: bits & 8 != 0,
        }
    }

    /// Exhaustive truth table: 3 handshake states × 2⁴ flags = 48
    /// combinations. `ready()` ⇔ Connected ∧ all four flags, and the
    /// reason is "ready" exactly when ready.
    #[test]
    fn ready_iff_connected_and_all_flags() {
        for hs in ALL_HANDSHAKES {
            for bits in 0..16u8 {
                let i = inputs(hs, bits);
                let expect = hs == JoinHandshake::Connected && bits == 15;
                assert_eq!(i.ready(), expect, "inputs: {i:?}");
                assert_eq!(i.block_reason() == "ready", expect, "inputs: {i:?}");
            }
        }
    }

    /// Reason names the FIRST blocker in bring-up order.
    #[test]
    fn block_reason_priority_order() {
        assert_eq!(
            inputs(JoinHandshake::Announced, 15).block_reason(),
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
            "mek-missing"
        );
        assert_eq!(
            inputs(JoinHandshake::Connected, 1 | 2).block_reason(),
            "caps-unreported"
        );
        assert_eq!(
            inputs(JoinHandshake::Connected, 1 | 2 | 4).block_reason(),
            "config-pending"
        );
        assert_eq!(inputs(JoinHandshake::Connected, 15).block_reason(), "ready");
    }

    #[test]
    fn update_emits_exactly_once_per_change() {
        let mut t = MediaReadyTracker::new();
        // First update always emits (seed → some reason).
        let first = t.update("c", "ch", |i| i.mek_present = true);
        assert_eq!(
            first,
            Some(MediaReadyTransition {
                ready: false,
                reason: "handshake-announced".into()
            })
        );
        // Same derived state → no emit, even though inputs changed.
        assert!(t
            .update("c", "ch", |i| i.local_caps_reported = true)
            .is_none());
        // Walk to ready: each step that changes the REASON emits.
        assert!(t
            .update("c", "ch", |i| i.handshake = JoinHandshake::Seen)
            .is_some());
        assert!(t
            .update("c", "ch", |i| i.handshake = JoinHandshake::Connected)
            .is_some());
        assert!(t.update("c", "ch", |i| i.roster_non_empty = true).is_some());
        let ready = t.update("c", "ch", |i| i.session_config_emitted = true);
        assert_eq!(
            ready,
            Some(MediaReadyTransition {
                ready: true,
                reason: "ready".into()
            })
        );
        assert!(t.is_ready("c", "ch"));
        // Re-applying the same input → no spurious re-emit.
        assert!(t.update("c", "ch", |i| i.roster_non_empty = true).is_none());
    }

    #[test]
    fn regression_re_emits_when_input_lost() {
        let mut t = MediaReadyTracker::new();
        t.update("c", "ch", |i| {
            *i = inputs(JoinHandshake::Connected, 15);
        });
        assert!(t.is_ready("c", "ch"));
        // Peer leaves → roster empties → must emit not-ready.
        let down = t.update("c", "ch", |i| i.roster_non_empty = false);
        assert_eq!(
            down,
            Some(MediaReadyTransition {
                ready: false,
                reason: "roster-empty".into()
            })
        );
    }

    #[test]
    fn clear_emits_only_when_previously_ready_or_unemitted() {
        let mut t = MediaReadyTracker::new();
        // Clearing an unknown slot: nothing.
        assert!(t.clear("c", "ch").is_none());
        // Slot that last emitted not-ready: clear is silent.
        t.update("c", "ch", |i| i.mek_present = true);
        assert!(t.clear("c", "ch").is_none());
        // Slot that was ready: clear emits left-channel.
        t.update("c", "ch", |i| {
            *i = inputs(JoinHandshake::Connected, 15);
        });
        assert_eq!(
            t.clear("c", "ch"),
            Some(MediaReadyTransition {
                ready: false,
                reason: "left-channel".into()
            })
        );
        assert_eq!(t.state("c", "ch"), (false, "not-in-voice".to_string()));
    }
}
