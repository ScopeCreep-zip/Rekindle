//! Daemon lifecycle state machine.
//!
//! Phase 5 of the decomposed-harvest plan hoisted the FSM into
//! `rekindle-lifecycle` so the Tauri shell can share it. The daemon
//! re-exports the shared types under their historical names
//! (`DaemonState`, `DaemonLifecycle`) so every existing callsite
//! compiles unchanged.
//!
//! `transition(...)` now returns `Result<LifecycleState, LifecycleError>`;
//! callsites that previously ignored the return value continue to work
//! via the `Result`'s `#[must_use]` warning being silenced with
//! `let _ = ...` (a deliberate signal that the daemon's existing
//! semantics — "log + ignore" on invalid edges — are preserved).

pub mod community_rpc;
pub mod dispatch;
pub mod event_router;
pub mod friend_inbox;
pub mod governance_rpc;
pub mod handler;

pub use rekindle_lifecycle::{
    AppLifecycle as DaemonLifecycle, LifecycleError, LifecycleState as DaemonState, TransportGuard,
};
