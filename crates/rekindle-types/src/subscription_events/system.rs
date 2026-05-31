//! System-level events — announcements, alerts, bootstrap, sync, kicked.

use serde::{Deserialize, Serialize};

/// System-level events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    /// A system message (community-wide announcement from operator).
    /// Triggered by: gossip `ControlPayload::SystemMessage`.
    Announcement {
        community: Option<String>,
        body: String,
        timestamp: u64,
    },
    /// A raid alert was activated or deactivated.
    /// Triggered by: gossip `ControlPayload::RaidAlert`.
    RaidAlert {
        community: String,
        active: bool,
    },
    /// A channel was locked or unlocked (lockdown mode).
    /// Triggered by: gossip `ControlPayload::ChannelLockdown`.
    ChannelLockdown {
        community: String,
        locked: bool,
    },
    /// We were kicked from a community.
    /// Triggered by: gossip `ControlPayload::KickedNotification`.
    Kicked {
        community: String,
    },
    /// A bootstrap request was received (operator processing a new joiner).
    /// Triggered by: gossip `ControlPayload::BootstrapRequest`.
    BootstrapRequested {
        community: String,
        joiner_pseudonym: String,
    },
    /// A bootstrap response was received (new joiner receiving initial state).
    /// Triggered by: gossip `ControlPayload::BootstrapResponse`.
    BootstrapReceived {
        community: String,
    },
    /// A sync request was received for a channel.
    /// Triggered by: gossip `ControlPayload::SyncRequest`.
    SyncRequested {
        community: String,
        channel: String,
        since_timestamp: u64,
    },
    /// A sync response was received with historical messages.
    /// Triggered by: gossip `ControlPayload::SyncResponse`.
    SyncReceived {
        community: String,
        channel: String,
        message_count: usize,
    },
    /// Phase 4 — local audit chain failed integrity check. The on-disk
    /// `audit_entries` table was tampered with, truncated, or corrupted
    /// at or after the given cursor. Triggered by: `audit_verify` Tauri
    /// command or boot-time chain check. Frontend surfaces as a typed
    /// `notification-event` toast so the user knows their device's
    /// integrity guarantees were violated.
    AuditChainBroken {
        cursor: u64,
    },
}
