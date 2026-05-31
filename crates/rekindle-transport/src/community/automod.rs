//! AutoMod configuration types for manifest subkey 9.
//!
//! Defines moderation rules evaluated on incoming messages at the
//! application layer (client-side enforcement).

use serde::{Deserialize, Serialize};

/// AutoMod configuration stored in governance manifest subkey 9.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModConfig {
    pub rules: Vec<AutoModRule>,
    pub raid_protection: RaidProtection,
}

/// A single auto-moderation rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModRule {
    pub rule_id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: AutoModTrigger,
    pub actions: Vec<AutoModAction>,
    #[serde(default)]
    pub exempt_roles: Vec<u32>,
    #[serde(default)]
    pub exempt_channels: Vec<String>,
}

/// What triggers an automod rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum AutoModTrigger {
    MentionSpam {
        limit: u8,
    },
    MessageSpam {
        per_interval: u8,
        interval_secs: u16,
    },
    JoinFlood {
        per_interval: u8,
        interval_secs: u16,
    },
    MessageSizeLimit {
        max_bytes: u32,
    },
}

/// What action to take when a rule triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum AutoModAction {
    BlockMessage,
    AlertModerators { channel_id: String },
    TimeoutMember { duration_secs: u64 },
    LogOnly,
}

/// Raid protection configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidProtection {
    pub enabled: bool,
    #[serde(default = "default_join_threshold")]
    pub join_threshold: u8,
    #[serde(default = "default_join_window_secs")]
    pub join_window_secs: u16,
    pub action: RaidAction,
}

/// What to do when a raid is detected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RaidAction {
    /// Pause new joins until manually cleared.
    #[default]
    PauseJoins,
    /// Lock all channels (non-admins cannot send).
    LockChannels,
    /// Kick new members that joined during the raid window.
    KickRecent,
}

fn default_join_threshold() -> u8 {
    10
}
fn default_join_window_secs() -> u16 {
    60
}
