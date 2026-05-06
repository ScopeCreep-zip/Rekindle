//! Typing indicator events — channel and DM contexts.
//!
//! Typing is ephemeral — indicators auto-expire after 5 seconds
//! without renewal. `Started` fires on first keystroke, `Stopped`
//! fires on expiry or explicit stop.

use serde::{Deserialize, Serialize};

/// Typing indicator events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypingEvent {
    /// Someone started typing.
    /// Triggered by: gossip `TypingIndicator`, `DmPayload::Typing { typing: true }`.
    Started {
        context: TypingContext,
        who: String,
    },
    /// Someone stopped typing (expired or explicit).
    /// Triggered by: expiry timer, `DmPayload::Typing { typing: false }`.
    Stopped {
        context: TypingContext,
        who: String,
    },
}

/// Where the typing is happening.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypingContext {
    /// Typing in a community channel.
    Channel { community: String, channel: String },
    /// Typing in a DM conversation.
    Dm { peer_key: String },
}
