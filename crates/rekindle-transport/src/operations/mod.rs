//! High-level orchestrated operations.
//!
//! Each submodule composes low-level transport primitives (DHT ops, send,
//! gossip, MEK crypto) into complete user-facing workflows. The CLI calls
//! these directly; the TUI calls them from spawned tasks.
//!
//! Every operation:
//! - Takes a `&TransportNode` plus the relevant session/config state
//! - Returns a typed result struct (not raw bytes)
//! - Has explicit timeout handling on all network calls
//! - Logs at `info` level on success, `warn` on recoverable failure

pub mod calls;
pub mod channel;
pub mod channel_admin;
pub mod community;
pub mod dm;
pub mod friend;
pub mod identity;
pub mod invites;
pub mod mek;
pub mod moderation;
pub mod presence;
pub mod roles;
pub mod voice;
