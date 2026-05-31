//! Phase 14 — call signaling dependency port + event surface.
//!
//! The `CallSignalingDeps` trait is what the rekindle-calls signaling
//! logic (handle_incoming_invite, accept/decline/ringing handlers,
//! group_signaling, ring_timer) calls into for every outside-world
//! operation: identity, registry mutation, transport (`app_message`
//! sends), voice session start/stop, persistence, frontend emit. The
//! src-tauri adapter (`services/calls_adapter.rs`, lands in 14.h)
//! implements them against `AppState` + `tauri::AppHandle` +
//! `message_service::send_to_peer_raw` + `services::voice::*`.
//!
//! Why split CallRegistry + GroupCallRegistry out of CallSignalingDeps:
//! - They're storage ports with concrete CRUD semantics (insert/get/
//!   remove/contains). Defining them as object-safe traits with sync
//!   methods keeps `Arc<dyn CallRegistry>` usable.
//! - Multiple modules (signaling, ring_timer, group_signaling) all
//!   need registry access; splitting avoids each grabbing the whole
//!   CallSignalingDeps.

pub mod deps;
pub mod event;
pub mod group_handlers; // Phase 14.e — group call signaling handlers.
pub mod handlers; // Phase 14.c — 1:1 call signaling handlers (invite/accept/decline/ringing).
pub mod outbound; // Phase 14.n — caller-side entry points (start_dm_call, etc.).
pub mod registry;

// Ring timers live on the deps trait:
//   - Receiver-side: `CallSignalingDeps::spawn_incoming_call_timeout`
//     (full sleep + remove + persist `missed_calls` + emit CallMissed)
//   - Caller-side: still on the src-tauri side in
//     `services::calls::ring_timer::spawn_dialing_timeout` (called from
//     `commands::calls::start_dm_call`, never traverses the crate)
// The earlier `crate::signaling::ring_timer` module was a stub that
// only removed the registry entry without emit/persist — replaced
// entirely by the trait method during the Phase 14.j audit.

#[cfg(test)]
mod tests; // Phase 14.j — handler-level regression coverage for audit-fixed paths.

pub use deps::CallSignalingDeps;
pub use event::CallSignalEvent;
pub use group_handlers::{
    handle_group_accept_received, handle_group_call_payload, handle_group_decline_received,
    handle_incoming_group_invite,
};
pub use handlers::{
    handle_accept_received, handle_decline_received, handle_incoming_invite,
    handle_ringing_received,
};
pub use outbound::{
    accept_dm_call, accept_group_call, decline_dm_call, decline_group_call, end_dm_call,
    end_group_call, start_dm_call, start_group_call,
};
pub use registry::{CallRegistry, GroupCallRegistry};
