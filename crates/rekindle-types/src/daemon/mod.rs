//! Daemon API contract — the command vocabulary between frontends and the daemon.
//!
//! These types define what the daemon accepts ([`DaemonRequest`]) and returns
//! ([`DaemonResponse`]). They are serialized to bytes by frontends (CLI, TUI,
//! Tauri, agent SDK) and deserialized by the daemon's [`FrameRouter`]
//! implementation. The transport layer carries them as opaque `&[u8]` — it
//! never inspects or deserializes these types.
//!
//! # Layout
//!
//! - [`request`] — `DaemonRequest` (90+ variants, one per daemon command)
//! - [`response`] — `DaemonResponse` (Ok/Error) + `ReadContext`
//! - [`AgentType`] — agent classification for registration

pub mod request;
pub mod response;

pub use request::DaemonRequest;
pub use response::{DaemonResponse, ReadContext};

use serde::{Deserialize, Serialize};

/// Agent type classification for the daemon's agent registry.
///
/// Determines rate limits, capability scope, and audit classification.
/// Carried in [`DaemonRequest::AgentRegister`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    /// Human user operating CLI, TUI, or Tauri.
    Human,
    /// Autonomous AI/LLM agent.
    AiLlm,
    /// Scheduled automation bot.
    Bot,
    /// Content filter (automod, spam, NSFW).
    Filter,
    /// Analysis service (metrics, sentiment).
    Analyzer,
    /// Protocol bridge (Matrix, IRC, Slack).
    Bridge,
    /// Internal system service (the daemon itself).
    System,
}

/// Registered agent metadata — name, type, and capabilities.
///
/// Application-level identity tracking. Transport-level identity
/// (Noise IK pubkeys) is managed by transport-ipc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub agent_type: AgentType,
    pub capabilities: Vec<String>,
}
