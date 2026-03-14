//! AutoMod enforcement for the coordinator.
//!
//! Evaluates incoming envelopes against the community's automod rules and
//! returns a decision: allow, block, timeout, or alert.

use std::collections::{HashMap, VecDeque};

use rekindle_protocol::dht::community::automod::{
    AutoModAction, AutoModConfig, AutoModRule, AutoModTrigger,
};
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_protocol::dht::community::types::{MemberSummary, RoleEntryV2};

/// Decision returned by automod rule evaluation.
#[derive(Debug, Clone)]
pub enum AutoModDecision {
    /// Message is allowed through.
    Allow,
    /// Message is blocked (not relayed).
    Block(String),
    /// Member should be timed out for the given duration.
    Timeout { duration_secs: u64, reason: String },
    /// Alert moderators in a specific channel.
    Alert { channel_id: String, reason: String },
}

/// Stateful automod enforcer that tracks rate counters per member.
pub struct AutoModEnforcer {
    config: AutoModConfig,
    /// Per-member message rate counters: pseudonym_key -> deque of timestamps.
    rate_counters: HashMap<String, VecDeque<u64>>,
    /// Per-(member, channel) slowmode tracker: last send timestamp.
    slowmode_counters: HashMap<(String, String), u64>,
}

impl AutoModEnforcer {
    /// Create a new enforcer with the given config.
    pub fn new(config: AutoModConfig) -> Self {
        Self {
            config,
            rate_counters: HashMap::new(),
            slowmode_counters: HashMap::new(),
        }
    }

    /// Hot-reload the config (e.g. when manifest subkey 9 changes).
    pub fn reload_config(&mut self, config: AutoModConfig) {
        self.config = config;
    }

    /// Check an envelope against all enabled automod rules.
    ///
    /// Returns the first non-Allow decision encountered, or `Allow` if all pass.
    /// `slowmode_seconds` comes from the channel's config and is checked separately.
    pub fn check_envelope(
        &mut self,
        sender: &MemberSummary,
        envelope: &CommunityEnvelope,
        roles: &[RoleEntryV2],
        channel_id: Option<&str>,
        slowmode_seconds: Option<u32>,
        now_secs: u64,
    ) -> AutoModDecision {
        // Administrators and MANAGE_COMMUNITY holders are always exempt
        if is_exempt(sender, roles) {
            return AutoModDecision::Allow;
        }

        // Clone rules to avoid borrowing self.config while calling &mut self methods
        let rules = self.config.rules.clone();
        for rule in &rules {
            if !rule.enabled {
                continue;
            }

            // Check role exemptions
            if rule
                .exempt_roles
                .iter()
                .any(|rid| sender.role_ids.contains(rid))
            {
                continue;
            }

            // Check channel exemptions
            if let Some(ch_id) = channel_id {
                if rule.exempt_channels.iter().any(|c| c == ch_id) {
                    continue;
                }
            }

            if let Some(decision) =
                self.evaluate_rule(rule, sender, envelope, channel_id, now_secs)
            {
                return decision;
            }
        }

        // Check slowmode (separate from rules — per-channel rate limit)
        if let (
            Some(ch_id),
            Some(sm_secs),
            CommunityEnvelope::ChatMessage { .. },
        ) = (channel_id, slowmode_seconds, envelope)
        {
            if sm_secs > 0 {
                if let Some(decision) = self.check_slowmode(sender, ch_id, sm_secs, now_secs) {
                    return decision;
                }
            }
        }

        AutoModDecision::Allow
    }

    fn evaluate_rule(
        &mut self,
        rule: &AutoModRule,
        sender: &MemberSummary,
        envelope: &CommunityEnvelope,
        _channel_id: Option<&str>,
        now_secs: u64,
    ) -> Option<AutoModDecision> {
        let triggered = match &rule.trigger {
            AutoModTrigger::MessageSpam {
                per_interval,
                interval_secs,
            } => {
                if !matches!(envelope, CommunityEnvelope::ChatMessage { .. }) {
                    return None;
                }
                self.check_message_rate(sender, *per_interval, *interval_secs, now_secs)
            }
            AutoModTrigger::MentionSpam { limit } => {
                // We can't see inside the encrypted message content, but we
                // can check the ciphertext size as a rough heuristic.
                // In practice, mention spam detection would need decrypted content.
                // For now, this trigger is a no-op for encrypted messages.
                let _ = limit;
                false
            }
            AutoModTrigger::MessageSizeLimit { max_bytes } => {
                if let CommunityEnvelope::ChatMessage { ciphertext, .. } = envelope {
                    ciphertext.len() > *max_bytes as usize
                } else {
                    false
                }
            }
            AutoModTrigger::JoinFlood { .. } => {
                // Join flood is handled by RaidDetector, not per-message rules.
                false
            }
        };

        if triggered {
            Some(Self::apply_actions(&rule.actions, &rule.name))
        } else {
            None
        }
    }

    fn check_message_rate(
        &mut self,
        sender: &MemberSummary,
        per_interval: u8,
        interval_secs: u16,
        now_secs: u64,
    ) -> bool {
        let counter = self
            .rate_counters
            .entry(sender.pseudonym_key.clone())
            .or_default();

        // Remove entries outside the window
        let window_start = now_secs.saturating_sub(u64::from(interval_secs));
        while counter.front().is_some_and(|&t| t < window_start) {
            counter.pop_front();
        }

        // Record this message
        counter.push_back(now_secs);

        counter.len() > usize::from(per_interval)
    }

    fn check_slowmode(
        &mut self,
        sender: &MemberSummary,
        channel_id: &str,
        slowmode_seconds: u32,
        now_secs: u64,
    ) -> Option<AutoModDecision> {
        let key = (sender.pseudonym_key.clone(), channel_id.to_string());
        if let Some(&last_send) = self.slowmode_counters.get(&key) {
            let elapsed = now_secs.saturating_sub(last_send);
            if elapsed < u64::from(slowmode_seconds) {
                let remaining = u64::from(slowmode_seconds) - elapsed;
                return Some(AutoModDecision::Block(format!(
                    "slowmode: wait {remaining}s"
                )));
            }
        }
        self.slowmode_counters.insert(key, now_secs);
        None
    }

    fn apply_actions(actions: &[AutoModAction], rule_name: &str) -> AutoModDecision {
        // Apply the most severe action
        let mut decision = AutoModDecision::Block(format!("automod: {rule_name}"));

        for action in actions {
            match action {
                AutoModAction::BlockMessage => {
                    decision = AutoModDecision::Block(format!("automod: {rule_name}"));
                }
                AutoModAction::TimeoutMember { duration_secs } => {
                    return AutoModDecision::Timeout {
                        duration_secs: *duration_secs,
                        reason: format!("automod: {rule_name}"),
                    };
                }
                AutoModAction::AlertModerators { channel_id } => {
                    return AutoModDecision::Alert {
                        channel_id: channel_id.clone(),
                        reason: format!("automod: {rule_name}"),
                    };
                }
                AutoModAction::LogOnly => {
                    decision = AutoModDecision::Allow;
                }
            }
        }

        decision
    }
}

/// Check if a member is exempt from automod (has ADMINISTRATOR or MANAGE_COMMUNITY).
fn is_exempt(member: &MemberSummary, roles: &[RoleEntryV2]) -> bool {
    for role_id in &member.role_ids {
        if let Some(role) = roles.iter().find(|r| r.id == *role_id) {
            let perms = Permissions::from_bits_truncate(role.permissions);
            if perms.contains(Permissions::ADMINISTRATOR)
                || perms.contains(Permissions::MANAGE_COMMUNITY)
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_member(pseudonym: &str, role_ids: Vec<u32>) -> MemberSummary {
        MemberSummary {
            pseudonym_key: pseudonym.into(),
            display_name: pseudonym.into(),
            role_ids,
            joined_at: 1000,
            subkey_index: 0,
            onboarding_complete: true,
            timeout_until: None,
        }
    }

    fn member_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 1,
            name: "Member".into(),
            color: 0,
            permissions: (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
            position: 1,
            hoist: false,
            mentionable: false,
        }
    }

    fn admin_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 3,
            name: "Admin".into(),
            color: 0,
            permissions: Permissions::ADMINISTRATOR.bits(),
            position: 3,
            hoist: false,
            mentionable: false,
        }
    }

    fn chat_message(ciphertext_len: usize) -> CommunityEnvelope {
        CommunityEnvelope::ChatMessage {
            channel_id: "ch_general".into(),
            message_id: "msg_001".into(),
            author_pseudonym: "alice".into(),
            ciphertext: vec![0u8; ciphertext_len],
            mek_generation: 1,
            timestamp: 5000,
            reply_to_id: None,
            lamport_ts: 0,
            sequence: 1,
        }
    }

    fn spam_config(per_interval: u8, interval_secs: u16) -> AutoModConfig {
        AutoModConfig {
            rules: vec![AutoModRule {
                rule_id: "rule_spam".into(),
                name: "Anti-spam".into(),
                enabled: true,
                trigger: AutoModTrigger::MessageSpam {
                    per_interval,
                    interval_secs,
                },
                actions: vec![AutoModAction::BlockMessage],
                exempt_roles: vec![],
                exempt_channels: vec![],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn allows_normal_message() {
        let mut enforcer = AutoModEnforcer::new(spam_config(5, 10));
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        let decision = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000);
        assert!(matches!(decision, AutoModDecision::Allow));
    }

    #[test]
    fn blocks_spam() {
        let mut enforcer = AutoModEnforcer::new(spam_config(3, 10));
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // Send 3 messages (within limit)
        for t in 0..3 {
            let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000 + t);
            assert!(matches!(d, AutoModDecision::Allow));
        }
        // 4th message should be blocked
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5003);
        assert!(matches!(d, AutoModDecision::Block(_)));
    }

    #[test]
    fn admin_exempt_from_spam() {
        let mut enforcer = AutoModEnforcer::new(spam_config(1, 10));
        let member = make_member("admin_user", vec![3]);
        let roles = vec![admin_role()];
        let envelope = chat_message(100);

        // Admin should be exempt even when exceeding rate
        for t in 0..5 {
            let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000 + t);
            assert!(matches!(d, AutoModDecision::Allow));
        }
    }

    #[test]
    fn blocks_oversized_message() {
        let config = AutoModConfig {
            rules: vec![AutoModRule {
                rule_id: "rule_size".into(),
                name: "Size limit".into(),
                enabled: true,
                trigger: AutoModTrigger::MessageSizeLimit { max_bytes: 1000 },
                actions: vec![AutoModAction::BlockMessage],
                exempt_roles: vec![],
                exempt_channels: vec![],
            }],
            ..Default::default()
        };
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];

        // Small message passes
        let small = chat_message(500);
        let d = enforcer.check_envelope(&member, &small, &roles, Some("ch_general"), None, 5000);
        assert!(matches!(d, AutoModDecision::Allow));

        // Large message blocked
        let large = chat_message(2000);
        let d = enforcer.check_envelope(&member, &large, &roles, Some("ch_general"), None, 5001);
        assert!(matches!(d, AutoModDecision::Block(_)));
    }

    #[test]
    fn exempt_channel_bypasses_rule() {
        let config = AutoModConfig {
            rules: vec![AutoModRule {
                rule_id: "rule_size".into(),
                name: "Size limit".into(),
                enabled: true,
                trigger: AutoModTrigger::MessageSizeLimit { max_bytes: 100 },
                actions: vec![AutoModAction::BlockMessage],
                exempt_roles: vec![],
                exempt_channels: vec!["ch_uploads".into()],
            }],
            ..Default::default()
        };
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(500);

        // Blocked in normal channel
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000);
        assert!(matches!(d, AutoModDecision::Block(_)));

        // Allowed in exempt channel
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_uploads"), None, 5001);
        assert!(matches!(d, AutoModDecision::Allow));
    }

    #[test]
    fn timeout_action() {
        let config = AutoModConfig {
            rules: vec![AutoModRule {
                rule_id: "rule_spam".into(),
                name: "Anti-spam".into(),
                enabled: true,
                trigger: AutoModTrigger::MessageSpam {
                    per_interval: 2,
                    interval_secs: 10,
                },
                actions: vec![AutoModAction::TimeoutMember {
                    duration_secs: 300,
                }],
                exempt_roles: vec![],
                exempt_channels: vec![],
            }],
            ..Default::default()
        };
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // 2 messages OK
        for t in 0..2 {
            let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000 + t);
            assert!(matches!(d, AutoModDecision::Allow));
        }
        // 3rd triggers timeout
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5002);
        assert!(matches!(d, AutoModDecision::Timeout { duration_secs: 300, .. }));
    }

    #[test]
    fn disabled_rule_skipped() {
        let mut config = spam_config(1, 10);
        config.rules[0].enabled = false;
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // Even 5 rapid messages should pass since rule is disabled
        for t in 0..5 {
            let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 5000 + t);
            assert!(matches!(d, AutoModDecision::Allow));
        }
    }

    #[test]
    fn rate_window_expires() {
        let mut enforcer = AutoModEnforcer::new(spam_config(3, 5));
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // Send 3 messages at t=1000..1002
        for t in 0..3 {
            let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 1000 + t);
            assert!(matches!(d, AutoModDecision::Allow));
        }
        // 4th at t=1003 is blocked (but still recorded in rate counter)
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 1003);
        assert!(matches!(d, AutoModDecision::Block(_)));

        // After full window expires (t=1009), all old entries pruned, counter fresh
        // window_start = 1009 - 5 = 1004, so entries [1000, 1001, 1002, 1003] all pruned
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), None, 1009);
        assert!(matches!(d, AutoModDecision::Allow));
    }

    #[test]
    fn slowmode_blocks_rapid_messages() {
        // No automod rules — only slowmode
        let config = AutoModConfig::default();
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // First message at t=1000 passes with 10s slowmode
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), Some(10), 1000);
        assert!(matches!(d, AutoModDecision::Allow));

        // Second message 5s later is blocked (within 10s slowmode)
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), Some(10), 1005);
        assert!(matches!(d, AutoModDecision::Block(_)));

        // Third message 11s after first passes
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), Some(10), 1011);
        assert!(matches!(d, AutoModDecision::Allow));
    }

    #[test]
    fn slowmode_zero_not_enforced() {
        let config = AutoModConfig::default();
        let mut enforcer = AutoModEnforcer::new(config);
        let member = make_member("alice", vec![1]);
        let roles = vec![member_role()];
        let envelope = chat_message(100);

        // slowmode_seconds=0 means no slowmode
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), Some(0), 1000);
        assert!(matches!(d, AutoModDecision::Allow));
        let d = enforcer.check_envelope(&member, &envelope, &roles, Some("ch_general"), Some(0), 1000);
        assert!(matches!(d, AutoModDecision::Allow));
    }
}
