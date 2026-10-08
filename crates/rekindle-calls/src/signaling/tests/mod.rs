//! Phase 14.j regression tests — handler-level integration coverage.
//!
//! These tests lock in the audit-fixed behaviors caught during the
//! Phase 13-style audit loop:
//!   - W14.1: `handle_accept_received` must invoke
//!     `deps.pre_stage_voice_channel()` BEFORE the
//!     `deps.start_voice_session(...)` await. Without this, voice
//!     packets arriving during the accept handler drop at dispatch.
//!   - W14.2: `handle_accept_received` must emit `CallConnected`
//!     carrying the call `kind` so the adapter maps
//!     `expected_local_camera = matches!(kind, Video)`. Without this,
//!     video calls never start local WebCodecs camera.
//!   - Group accept: `handle_group_accept_received` must transition
//!     `GroupCallState.status` to `Active` on the first accept (not
//!     just emit `GroupCallConnected`).

mod mocks;

mod display_name;
mod handlers;
