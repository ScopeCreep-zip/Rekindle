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
        .filter_map(|candidate| {
            stage_host_score(channel_id, candidate).map(|score| (candidate, score))
        })
        .min_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(candidate, _)| candidate.clone())
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
