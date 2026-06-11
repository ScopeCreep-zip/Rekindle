#![recursion_limit = "512"]
//! Rekindle-node daemon library.
//!
//! This crate implements the rekindle-node daemon: orchestrates
//! transport-ipc (local IPC), rekindle-chat (business logic),
//! rekindle-storage (persistence), and rekindle-transport (Veilid network).
//!
//! # Architecture
//!
//! ```text
//! CLI/TUI/Tauri ←→ transport-ipc (AF_UNIX) ←→ rekindle-node ←→ rekindle-chat ←→ Veilid
//! ```
//!
//! # Module Organization
//!
//! - [`daemon`] — Lifecycle state machine (STOPPED → OPERATIONAL) + dispatch.
//! - [`routing`] — DaemonRouter: bridges transport-ipc FrameRouter to dispatch.
//! - [`subscriptions`] — Per-connection event fan-out registry.
//! - [`journal`] — Cursor-based event replay for reconnection.
//! - [`idempotency`] — Exactly-once request processing cache.
//! - [`state`] — XDG path resolution.
//! - [`validation`] — Input validation for the daemon security boundary.
//!
//! Key management (keypair generation, persistence, tamper detection) lives
//! in the [`rekindle-keys`] crate, shared with `rekindle-client`.

#![deny(unsafe_code)]

pub mod daemon;
pub mod routing;
pub mod subscriptions;
pub mod journal;
pub mod idempotency;
pub mod state;
pub mod validation;

/// Run the rekindle daemon. Process entry point for `rekindle node start`.
pub async fn run_daemon() -> anyhow::Result<()> {
    daemon::run::run_daemon().await
}
