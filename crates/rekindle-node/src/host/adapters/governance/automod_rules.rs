//! A community's automod rules (plan E3.1 F3, D26), on both hosts: each
//! set or delete is one `GovernanceEntry::AutoModRule` (LWW per rule id)
//! written through `rekindle_governance_runtime::apply::write_entry`, then
//! the projected governance is persisted. Every peer's ingest gate compiles
//! the merged rules once per rule-set version (`rekindle_channel::automod`).

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dto::community::AutoModRuleView;
use rekindle_types::governance::{AutoModAction, GovernanceEntry};
use rekindle_types::permissions;

use super::HostGovernance;

/// A rule as the moderator wrote it; no `rule_id` creates one.
pub(crate) struct AutoModRuleEdit {
    pub rule_id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub keywords: Vec<String>,
    pub regex_patterns: Vec<String>,
    pub action: AutoModAction,
}

fn rule_id(hex_id: &str) -> Result<[u8; 16], GovernanceRuntimeError> {
    hex::decode(hex_id)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .ok_or_else(|| GovernanceRuntimeError::Adapter(format!("invalid rule id {hex_id:?}")))
}

fn trigger_json(keywords: &[String], regex_patterns: &[String]) -> String {
    serde_json::json!({ "keywords": keywords, "regexPatterns": regex_patterns }).to_string()
}

impl HostGovernance {
    /// The community's enabled rules, by name.
    ///
    /// # Errors
    /// The community is not loaded.
    pub(crate) fn automod_rules(
        &self,
        community_id: &str,
    ) -> Result<Vec<AutoModRuleView>, GovernanceRuntimeError> {
        let gov = self
            .governance_state(community_id)
            .ok_or_else(|| GovernanceRuntimeError::Adapter("governance state not loaded".into()))?;
        Ok(rekindle_channel::automod::rules_of(&gov)
            .into_iter()
            .map(|rule| AutoModRuleView {
                rule_id: rule.rule_id,
                name: rule.name,
                enabled: rule.enabled,
                keywords: rule.keywords,
                regex_patterns: rule.regex_patterns,
                action: rule.action,
                lamport: rule.lamport,
            })
            .collect())
    }

    /// Write `edit`; answers the rule's id.
    ///
    /// # Errors
    /// We lack `MANAGE_COMMUNITY`, the id is not 16 bytes of hex, the rule
    /// is refused (its regexes or quotas) or the write failed.
    pub(crate) async fn set_automod_rule(
        &self,
        community_id: &str,
        edit: AutoModRuleEdit,
    ) -> Result<String, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_COMMUNITY)?;
        let id = if let Some(hex_id) = edit.rule_id.as_deref() {
            rule_id(hex_id)?
        } else {
            let mut id = [0u8; 16];
            self.rng().fill(&mut id);
            id
        };
        let entry = GovernanceEntry::AutoModRule {
            rule_id: id,
            name: edit.name,
            enabled: edit.enabled,
            trigger_json: trigger_json(&edit.keywords, &edit.regex_patterns),
            action: edit.action,
            lamport: self.next_governance_lamport(community_id)?,
        };
        rekindle_governance_runtime::apply::write_entry(self, community_id, entry).await?;
        self.persist_governance_snapshot(community_id).await;
        Ok(hex::encode(id))
    }

    /// Turn the rule `hex_id` off: the same rule, disabled, at a newer
    /// lamport.
    ///
    /// # Errors
    /// We lack `MANAGE_COMMUNITY`, there is no such rule, or the write
    /// failed.
    pub(crate) async fn delete_automod_rule(
        &self,
        community_id: &str,
        hex_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_COMMUNITY)?;
        let id = rule_id(hex_id)?;
        let existing = self
            .governance_state(community_id)
            .and_then(|gov| gov.automod_rules.get(&id).cloned())
            .ok_or_else(|| GovernanceRuntimeError::Adapter("automod rule not found".into()))?;
        let entry = GovernanceEntry::AutoModRule {
            rule_id: id,
            name: existing.name,
            enabled: false,
            trigger_json: existing.trigger_json,
            action: existing.action,
            lamport: self.next_governance_lamport(community_id)?,
        };
        rekindle_governance_runtime::apply::write_entry(self, community_id, entry).await?;
        self.persist_governance_snapshot(community_id).await;
        Ok(())
    }
}
