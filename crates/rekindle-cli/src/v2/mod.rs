//! Rekindle CLI v2 — complete rewrite against the restructured crate architecture.
//!
//! Architecture invariant: this crate is an IPC client ONLY. No business logic,
//! no crypto, no storage access, no transport calls. It sends `IpcRequest`
//! variants over the Noise IK encrypted Unix socket and renders responses.

pub mod entrypoint;
pub mod error;
pub mod helpers;
pub mod output;
pub mod transport;
pub mod cli;
pub mod commands;
pub mod config;
pub mod patch;
pub mod search;

#[cfg(feature = "tui")]
pub mod tui;
#[cfg(feature = "tui")]
pub mod views;
