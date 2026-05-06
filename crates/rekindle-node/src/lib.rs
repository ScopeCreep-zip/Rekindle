// Veilid's deeply nested `#[instrument]` attributes on network operations
// produce future types that exceed the default recursion limit (128) when
// spawned via `tokio::spawn`. This matches the transport crate's limit.
#![recursion_limit = "512"]
//! Rekindle-node daemon library.
//!
//! This crate implements the rekindle-node daemon: an IPC bus server that owns
//! the Veilid node, manages persistent state, and serves CLI/TUI/Tauri frontends
//! plus AI/LLM agents, automation bots, filters, and bridges.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │              rekindle-node                    │
//! │                                              │
//! │  ipc/     — Encrypted IPC bus (Noise IK)    │
//! │  daemon/  — Lifecycle state machine          │
//! │  state/   — Persistent state management      │
//! └─────────────────────────────────────────────┘
//!              │
//!              ▼
//! ┌─────────────────────────────────────────────┐
//! │         rekindle-transport                    │
//! │  (sole Veilid boundary — no other crate      │
//! │   imports veilid-core)                        │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Module Organization
//!
//! - [`ipc`] — Zero Veilid knowledge. Pure encrypted IPC bus.
//! - [`daemon`] — Lifecycle state machine (STOPPED → OPERATIONAL).
//! - [`state`] — Session, config, and path management.
//!
//! # Veilid Boundary
//!
//! This crate depends on `rekindle-transport`, NOT on `veilid-core`.
//! All Veilid operations are accessed through `rekindle_transport::TransportNode`,
//! `rekindle_transport::operations::*`, and `rekindle_transport::Session`.

#![forbid(unsafe_code)]

pub mod ipc;
pub mod daemon;
pub mod state;
pub mod validation;
