//! Phase 14.l — voice session orchestration.
//!
//! Ports `src-tauri/services/voice/session.rs::start_session` into the
//! crate, parameterised over [`VoiceSessionDeps`]. The orchestration
//! logic — order of operations, peer-route fallback decision, §10.6
//! capabilities broadcast, frontend emit — lives here. Engine/cpal
//! setup + AppState mutation stay in the adapter via deps methods
//! (init_voice_session, spawn_voice_loops, resolve_peer_route, etc.).
//!
//! Subsequent sub-phases port restart_loops, start/stop_mcu_loop,
//! shutdown, and device_monitor into sibling modules.

pub mod device_monitor;
pub mod local_controls;
pub mod mcu;
pub mod restart;
pub mod shutdown;
pub mod start;

pub use local_controls::{
    change_audio_devices, join_voice_channel, leave_voice, set_local_deafen, set_local_mute,
    set_voice_mode,
};
pub use mcu::{start_mcu_loop, stop_mcu_loop};
pub use restart::restart_loops;
pub use shutdown::shutdown_voice;
pub use start::start_session;
