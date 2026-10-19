//! Typing indicator events — channel and DM contexts.
//!
//! Typing is ephemeral and decided in the backend: a start lapses after
//! 5 seconds without a refresh (`rekindle_events::typing`). `Started`
//! fires once per typing burst, `Stopped` on the typist's stop, their
//! message, or the lapse; frontends keep no timer.

use serde::{Deserialize, Serialize};

/// Typing indicator events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TypingEvent {
    /// Someone started typing.
    /// Triggered by: gossip `TypingIndicator`, a 1:1 `TypingIndicator { typing: true }`.
    Started { context: TypingContext, who: String },
    /// Someone stopped typing (expired or explicit).
    /// Triggered by: the backend's expiry, a 1:1 `TypingIndicator { typing: false }`,
    /// or the typist's message arriving.
    Stopped { context: TypingContext, who: String },
}

/// Where the typing is happening.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TypingContext {
    /// Typing in a community channel.
    Channel { community: String, channel: String },
    /// Typing in a DM conversation.
    Dm { peer_key: String },
}
