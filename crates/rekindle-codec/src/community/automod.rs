//! AutoMod configuration types for manifest subkey 9.
//!
//! Defines moderation rules that the coordinator enforces on incoming envelopes
//! before relaying them to other members.

use serde::{Deserialize, Serialize};

/// AutoMod configuration stored in manifest subkey 9.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModConfig {
    /// Ordered list of moderation rules. Evaluated top-to-bottom.
    pub rules: Vec<AutoModRule>,
    /// Raid protection settings.
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
    /// Role IDs exempt from this rule.
    #[serde(default)]
    pub exempt_roles: Vec<u32>,
    /// Channel IDs exempt from this rule.
    #[serde(default)]
    pub exempt_channels: Vec<String>,
}

/// What triggers an automod rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum AutoModTrigger {
    /// Too many @mentions in a single message.
    MentionSpam {
        /// Maximum number of mentions before triggering.
        limit: u8,
    },
    /// Too many messages in a time window.
    MessageSpam {
        /// Maximum messages per interval before triggering.
        per_interval: u8,
        /// Time window in seconds.
        interval_secs: u16,
    },
    /// Too many joins in a time window (member-level, not message-level).
    JoinFlood {
        /// Maximum joins per interval before triggering.
        per_interval: u8,
        /// Time window in seconds.
        interval_secs: u16,
    },
    /// Message exceeds size limit.
    MessageSizeLimit {
        /// Maximum ciphertext size in bytes.
        max_bytes: u32,
    },
}

/// What action to take when a rule triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum AutoModAction {
    /// Block the message from being relayed.
    BlockMessage,
    /// Alert moderators in a specific channel.
    AlertModerators { channel_id: String },
    /// Timeout the member for a duration.
    TimeoutMember { duration_secs: u64 },
    /// Log the event without taking action.
    LogOnly,
}

/// Raid protection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidProtection {
    /// Whether raid protection is enabled.
    pub enabled: bool,
    /// Number of joins in a 60-second window to trigger raid mode.
    pub join_rate_threshold: u8,
    /// Actions to take when raid is detected.
    pub actions: Vec<RaidAction>,
    /// Seconds before automatically resolving raid mode (0 = manual only).
    pub auto_resolve_secs: u32,
}

impl Default for RaidProtection {
    fn default() -> Self {
        Self {
            enabled: false,
            join_rate_threshold: 10,
            actions: Vec::new(),
            auto_resolve_secs: 300,
        }
    }
}

/// Actions taken during a detected raid.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RaidAction {
    /// Temporarily stop accepting new invites.
    PauseInvites,
    /// Restrict new members to read-only.
    RestrictNewMembers,
    /// Alert community owners.
    AlertOwners,
    /// Lock all channels (deny SEND_MESSAGES for non-admins).
    LockdownChannels,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automod_config_default() {
        let config = AutoModConfig::default();
        assert!(config.rules.is_empty());
        assert!(!config.raid_protection.enabled);
    }

    #[test]
    fn automod_config_serde() {
        let config = AutoModConfig {
            rules: vec![AutoModRule {
                rule_id: "rule_01".into(),
                name: "Anti-spam".into(),
                enabled: true,
                trigger: AutoModTrigger::MessageSpam {
                    per_interval: 5,
                    interval_secs: 10,
                },
                actions: vec![
                    AutoModAction::BlockMessage,
                    AutoModAction::TimeoutMember { duration_secs: 300 },
                ],
                exempt_roles: vec![3, 4],
                exempt_channels: vec![],
            }],
            raid_protection: RaidProtection {
                enabled: true,
                join_rate_threshold: 15,
                actions: vec![RaidAction::PauseInvites, RaidAction::AlertOwners],
                auto_resolve_secs: 600,
            },
        };

        let json = serde_json::to_string(&config).unwrap();
        let back: AutoModConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rules.len(), 1);
        assert_eq!(back.rules[0].rule_id, "rule_01");
        assert!(back.raid_protection.enabled);
        assert_eq!(back.raid_protection.join_rate_threshold, 15);
    }

    #[test]
    fn trigger_variants_serde() {
        let triggers = vec![
            AutoModTrigger::MentionSpam { limit: 5 },
            AutoModTrigger::MessageSpam {
                per_interval: 10,
                interval_secs: 30,
            },
            AutoModTrigger::JoinFlood {
                per_interval: 8,
                interval_secs: 60,
            },
            AutoModTrigger::MessageSizeLimit { max_bytes: 4096 },
        ];

        for trigger in &triggers {
            let json = serde_json::to_string(trigger).unwrap();
            let back: AutoModTrigger = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn action_variants_serde() {
        let actions = vec![
            AutoModAction::BlockMessage,
            AutoModAction::AlertModerators {
                channel_id: "ch_mod".into(),
            },
            AutoModAction::TimeoutMember { duration_secs: 600 },
            AutoModAction::LogOnly,
        ];

        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let back: AutoModAction = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn raid_action_variants_serde() {
        let actions = vec![
            RaidAction::PauseInvites,
            RaidAction::RestrictNewMembers,
            RaidAction::AlertOwners,
            RaidAction::LockdownChannels,
        ];

        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let back: RaidAction = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }
}
