// Veilid's deeply nested `#[instrument]` attributes on network operations
// produce future types that exceed the default recursion limit (128) when
// spawned via `tokio::spawn`. This matches the transport crate's limit.
#![recursion_limit = "512"]
//! Rekindle-node daemon library.
//!
//! This crate implements `rekindled`, the one backend host: it owns the Veilid
//! node and persistent state, and serves the CLI/TUI/Tauri frontends (plus
//! agents, bots and bridges) over the `rekindle-ipc` bus.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │              rekindle-node                    │
//! │                                              │
//! │  host/    — `rekindled` composition root     │
//! │  daemon/  — Lifecycle state machine          │
//! │  state/   — Persistent state management      │
//! └─────────────────────────────────────────────┘
//!              │
//!              ▼
//! ┌─────────────────────────────────────────────┐
//! │         rekindle-transport                    │
//! │  (the daemon's Veilid adapter, over           │
//! │   rekindle-protocol's record pool and routes)  │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Module Organization
//!
//! - [`host`] — The `rekindled` composition root: node lock, bus, workers.
//! - [`daemon`] — Lifecycle state machine (STOPPED → OPERATIONAL).
//! - [`state`] — Session, config, and path management.
//!
//! # Veilid Boundary
//!
//! This crate names no `veilid-core` type: every Veilid operation goes through
//! `rekindle_transport::TransportNode`, `rekindle_transport::operations::*` and
//! `rekindle_transport::Session`. It does link veilid-core, through
//! `rekindle-transport`, and is one of the crates allowed to (ADR 0014: the
//! boundary is transitive linkage; the calls themselves live in
//! `rekindle-protocol`, rule B22).

#![forbid(unsafe_code)]
// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]

pub mod daemon;
pub mod host;
pub mod state;
pub mod validation;
