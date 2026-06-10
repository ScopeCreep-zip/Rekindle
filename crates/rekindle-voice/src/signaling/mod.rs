//! Phase 14.k — community voice signaling handlers.
//!
//! Handles voice-related `ControlPayload` variants received via the
//! community gossip mesh (architecture §10.7 / §10.9 / §10.10): voice
//! join/leave, mode switches (mesh↔MCU), stage updates, speak
//! requests, voice mute/deafen, soundboard play. Parameterized over
//! [`VoiceSignalingDeps`] so the crate stays free of AppState, Tauri,
//! and the cross-subsystem `services::community::*` functions until
//! Phases 17 (MEK rotation), 19 (channel persistence), and 20 (gossip
//! mesh) own those domains natively.
//!
//! Module layout — split to keep each file under the plan's ≤500 LoC
//! cap and to give each module a single noun-phrase responsibility:
//!
//! - `deps`: `VoiceSignalingDeps` trait + `CommunityVoiceEvent` +
//!   `StageChannelInfo` + permission constants.
//! - `dispatcher`: `handle_voice_signaling` entry point + the
//!   mode-switch broadcast helper shared by presence.
//! - `presence`: voice_join / voice_leave / voice_roster: peer
//!   add/remove + §10.2 mesh↔MCU auto-switch.
//! - `stage`: stage_update / speak_request / speak_response +
//!   `reconcile_stage_transport` (§10.7 always-MCU).
//! - `mute`: voice_mute / voice_deafen + soundboard_play (§9.3
//!   reader-validates + §10.9 USE_SOUNDBOARD).
//!
//! Pre-Phase-14.k these handlers lived in
//! `src-tauri/services/voice/signaling.rs` (858 LoC of orchestration
//! mixed with AppState/Tauri coupling).

pub mod deps;
pub(crate) mod dispatcher;
pub(crate) mod mute;
pub(crate) mod presence;
pub mod roster_reconcile;
pub(crate) mod stage;

pub use deps::{
    perms, CommunityVoiceEvent, StageChannelInfo, VoiceRosterParticipant, VoiceSignalingDeps,
};
pub use dispatcher::handle_voice_signaling;
pub use roster_reconcile::{
    compute_roster_reconcile, reconcile_from_presence, PresencePeerView, ReconcileAdd,
    ReconcilePlan, JOIN_GRACE_SECS,
};
pub use stage::{
    request_to_speak, respond_to_speak_request, server_deafen_member, server_mute_member,
};
