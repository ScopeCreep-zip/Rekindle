//! Phase 14 — pure voice-topology decisions.
//!
//! These are the deterministic mode-decision and host-selection
//! helpers extracted from `src-tauri/services/voice/signaling.rs`. The
//! community gossip orchestrator (which talks to AppState, MEK rotation,
//! mesh broadcast, etc.) stays in src-tauri per the Path-2 decision —
//! but the math it uses to decide *whether* to switch mode and *who*
//! should host lives here, free of any Tauri/AppState/MEK-rotation
//! coupling.
//!
//! Architecture references:
//! - §10.2 lines 2017-2019 — full-mesh ≤4 members, MCU ≥5 members.
//! - §10.7 — stage channels always operate in MCU/relay mode.

use crate::VoiceMode;

/// What the orchestrator should do with the transport after a voice
/// roster change. Returned by [`decide_mode_after_join`] /
/// [`decide_mode_after_leave`] so the caller can apply the decision
/// (transport mode-flip, MCU loop start/stop, mesh broadcast) without
/// re-deriving the conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeDecision {
    /// No change required.
    NoChange,
    /// Switch to full-mesh mode. The orchestrator stops the MCU loop
    /// (if running) and broadcasts a `VoiceModeSwitch { mode: "mesh" }`.
    SwitchToMesh,
    /// Switch to MCU/SFU mode with the named host. The orchestrator
    /// flips transport mode, broadcasts `VoiceModeSwitch { mode: "mcu" }`,
    /// and starts the MCU loop if `host == my_pseudonym`.
    SwitchToMcu { host: String },
}

/// Architecture §10.2: full-mesh below 5 total participants, MCU above.
/// `peer_count` excludes self — total = `peer_count + 1`.
///
/// Stage channels (`is_stage = true`) skip this auto-switch entirely;
/// stage topology is reconciled separately by `select_stage_host`.
#[must_use]
pub fn decide_mode_after_join(
    peer_count: usize,
    current_mode: &VoiceMode,
    is_stage: bool,
    elected_host: Option<&str>,
) -> ModeDecision {
    if is_stage {
        return ModeDecision::NoChange;
    }
    // Total ≥ 5 (peer_count ≥ 4 excludes self) and we're still in
    // mesh → elect MCU host.
    if peer_count >= 4 && matches!(current_mode, VoiceMode::Mesh) {
        if let Some(host) = elected_host {
            return ModeDecision::SwitchToMcu {
                host: host.to_string(),
            };
        }
    }
    ModeDecision::NoChange
}

/// Architecture §10.2: on a leave, fall back to mesh if either (a) the
/// MCU host left, or (b) we dropped below the 5-total threshold.
///
/// `host_left` is whether the departing peer is the current MCU host.
/// `elected_host` is the next candidate if the orchestrator already
/// ran re-election (only consulted when `host_left` and we're still
/// at MCU scale).
#[must_use]
pub fn decide_mode_after_leave(
    peer_count: usize,
    current_mode: &VoiceMode,
    is_stage: bool,
    host_left: bool,
    elected_host: Option<&str>,
) -> ModeDecision {
    if is_stage {
        return ModeDecision::NoChange;
    }
    match current_mode {
        VoiceMode::Mesh => ModeDecision::NoChange,
        VoiceMode::Mcu { .. } => {
            if host_left {
                if peer_count >= 4 {
                    if let Some(host) = elected_host {
                        return ModeDecision::SwitchToMcu {
                            host: host.to_string(),
                        };
                    }
                }
                ModeDecision::SwitchToMesh
            } else if peer_count < 4 {
                ModeDecision::SwitchToMesh
            } else {
                ModeDecision::NoChange
            }
        }
    }
}

/// Score a candidate stage speaker via BLAKE3-XOR distance to a
/// channel-id hash. Lowest score wins (deterministic across peers,
/// resistant to manipulation since every peer can verify locally).
#[must_use]
pub fn stage_host_score(channel_id: &str, speaker_hex: &str) -> Option<[u8; 32]> {
    let speaker_bytes: [u8; 32] = hex::decode(speaker_hex).ok()?.try_into().ok()?;
    let channel_hash = blake3::hash(channel_id.as_bytes());
    let mut score = [0u8; 32];
    for (index, byte) in score.iter_mut().enumerate() {
        *byte = speaker_bytes[index] ^ channel_hash.as_bytes()[index];
    }
    Some(score)
}

/// Pick the deterministic stage host (lowest XOR-distance score).
/// Returns `None` when no candidate is decodable.
#[must_use]
pub fn select_stage_host(channel_id: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .filter_map(|candidate| stage_host_score(channel_id, candidate).map(|score| (candidate, score)))
        .min_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(candidate, _)| candidate.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh() -> VoiceMode {
        VoiceMode::Mesh
    }

    fn mcu(host: &str) -> VoiceMode {
        VoiceMode::Mcu {
            host_pseudonym: host.to_string(),
        }
    }

    #[test]
    fn join_below_threshold_stays_mesh() {
        // peer_count = 3 → total = 4, still mesh per §10.2.
        assert_eq!(
            decide_mode_after_join(3, &mesh(), false, Some("alice")),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn join_at_threshold_switches_to_mcu() {
        // peer_count = 4 → total = 5, switch to MCU.
        assert_eq!(
            decide_mode_after_join(4, &mesh(), false, Some("alice")),
            ModeDecision::SwitchToMcu {
                host: "alice".into()
            },
        );
    }

    #[test]
    fn join_at_threshold_without_elected_host_no_change() {
        // Election failed — orchestrator handles the warn path.
        assert_eq!(
            decide_mode_after_join(4, &mesh(), false, None),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn join_above_threshold_in_mcu_no_change() {
        assert_eq!(
            decide_mode_after_join(5, &mcu("alice"), false, Some("bob")),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn stage_channel_join_never_switches() {
        assert_eq!(
            decide_mode_after_join(10, &mesh(), true, Some("alice")),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn leave_mesh_no_change() {
        assert_eq!(
            decide_mode_after_leave(2, &mesh(), false, false, None),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn leave_mcu_below_threshold_switches_to_mesh() {
        assert_eq!(
            decide_mode_after_leave(3, &mcu("alice"), false, false, None),
            ModeDecision::SwitchToMesh,
        );
    }

    #[test]
    fn leave_mcu_host_left_with_reelection() {
        assert_eq!(
            decide_mode_after_leave(4, &mcu("alice"), false, true, Some("bob")),
            ModeDecision::SwitchToMcu {
                host: "bob".into()
            },
        );
    }

    #[test]
    fn leave_mcu_host_left_below_threshold_to_mesh() {
        assert_eq!(
            decide_mode_after_leave(3, &mcu("alice"), false, true, Some("bob")),
            ModeDecision::SwitchToMesh,
        );
    }

    #[test]
    fn leave_mcu_host_left_no_election_to_mesh() {
        assert_eq!(
            decide_mode_after_leave(4, &mcu("alice"), false, true, None),
            ModeDecision::SwitchToMesh,
        );
    }

    #[test]
    fn leave_mcu_above_threshold_non_host_no_change() {
        assert_eq!(
            decide_mode_after_leave(5, &mcu("alice"), false, false, None),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn stage_channel_leave_never_switches() {
        assert_eq!(
            decide_mode_after_leave(0, &mcu("alice"), true, true, None),
            ModeDecision::NoChange,
        );
    }

    #[test]
    fn stage_host_score_deterministic() {
        let a = stage_host_score("c0ffee", "ab".repeat(32).as_str()).unwrap();
        let b = stage_host_score("c0ffee", "ab".repeat(32).as_str()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn stage_host_score_rejects_bad_hex() {
        assert!(stage_host_score("c0ffee", "nothex").is_none());
        assert!(stage_host_score("c0ffee", "deadbeef").is_none()); // wrong length
    }

    #[test]
    fn select_stage_host_picks_lowest_score() {
        let candidates = vec!["aa".repeat(32), "bb".repeat(32), "cc".repeat(32)];
        let host = select_stage_host("channel-1", &candidates).unwrap();
        // Whichever is lowest XOR to BLAKE3("channel-1") wins. Verify
        // by computing all scores and confirming the picked one is
        // minimum.
        let picked_score = stage_host_score("channel-1", &host).unwrap();
        for c in &candidates {
            let s = stage_host_score("channel-1", c).unwrap();
            assert!(picked_score <= s, "picked host should have lowest score");
        }
    }

    #[test]
    fn select_stage_host_empty_returns_none() {
        assert!(select_stage_host("any", &[]).is_none());
    }

    #[test]
    fn select_stage_host_filters_undecodable() {
        let candidates = vec!["nothex".into(), "aa".repeat(32)];
        let host = select_stage_host("c", &candidates).unwrap();
        assert_eq!(host, "aa".repeat(32));
    }
}
