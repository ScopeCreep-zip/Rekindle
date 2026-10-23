//! Automod rules (plan E3.1 D26): read from the merged
//! `GovernanceState.automod_rules`, compiled once per rule set (regexes and
//! lowercased keywords) and matched against received post bodies by the
//! ingest gate. The compiled set is cached on the adapter per community and
//! keyed by the rules' version fingerprint, so a projection that changes a
//! rule (every change is a new LWW lamport) recompiles it, and nothing else
//! does.

use regex::Regex;
use std::sync::Arc;

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::GovernanceState;
use rekindle_types::governance::AutoModAction;
use serde::Deserialize;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TriggerConfig {
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    regex_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AutoModRuleInfo {
    pub rule_id: String,
    pub name: String,
    pub enabled: bool,
    pub keywords: Vec<String>,
    pub regex_patterns: Vec<String>,
    pub action: AutoModAction,
    pub lamport: u64,
}

/// Compiled regex + lowered keyword set for a single rule. Returned by
/// `compile_rules` and cached on the adapter via `AutoModCompiledCache`.
#[derive(Debug, Clone)]
pub struct CompiledAutoModRule {
    pub rule_id: [u8; 16],
    pub name: String,
    pub keywords_lower: Vec<String>,
    pub regexes: Vec<Regex>,
    pub action: AutoModAction,
}

/// One community's compiled rule set. `fingerprint` is the sorted
/// `(rule_id, lamport)` set it was compiled from; `rules` holds the enabled
/// rules in rule-id order, or why they could not be compiled (the gate drops
/// every post while that holds, and the error is cached like a success, so
/// a broken rule is not recompiled per post either).
#[derive(Debug, Clone)]
pub struct AutoModCompiledCache {
    pub fingerprint: Vec<([u8; 16], OpStamp)>,
    pub rules: Result<Vec<CompiledAutoModRule>, String>,
}

impl AutoModCompiledCache {
    /// Compile `governance`'s rules.
    #[must_use]
    pub fn compile(governance: &GovernanceState) -> Self {
        Self {
            fingerprint: rule_fingerprint(governance),
            rules: compile_rules(governance).map_err(|e| e.to_string()),
        }
    }
}

/// `community_id`'s compiled rules: the adapter's cached set while its
/// fingerprint matches `governance`, else a fresh compile, cached.
pub fn compiled_rules<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    governance: &GovernanceState,
) -> Arc<AutoModCompiledCache> {
    let fingerprint = rule_fingerprint(governance);
    if let Some(cached) = deps.automod_compiled_cache_get(community_id) {
        if cached.fingerprint == fingerprint {
            return cached;
        }
    }
    let compiled = Arc::new(AutoModCompiledCache::compile(governance));
    deps.automod_compiled_cache_set(community_id, Arc::clone(&compiled));
    compiled
}

pub fn list_rules<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<AutoModRuleInfo>, ChannelError> {
    let governance = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    Ok(rules_of(&governance))
}

pub fn get_rule<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    rule_id: &[u8; 16],
) -> Result<AutoModRuleInfo, ChannelError> {
    let governance = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let rule = governance
        .automod_rules
        .get(rule_id)
        .ok_or_else(|| ChannelError::Adapter("automod rule not found".into()))?;
    let trigger: TriggerConfig = serde_json::from_str(&rule.trigger_json)
        .map_err(|e| ChannelError::Adapter(format!("invalid automod rule trigger: {e}")))?;
    Ok(AutoModRuleInfo {
        rule_id: hex::encode(rule_id),
        name: rule.name.clone(),
        enabled: rule.enabled,
        keywords: trigger.keywords,
        regex_patterns: trigger.regex_patterns,
        action: rule.action,
        lamport: rule.stamp.lamport,
    })
}

fn compile_rules(governance: &GovernanceState) -> Result<Vec<CompiledAutoModRule>, ChannelError> {
    let mut rules = Vec::new();
    for (rule_id, rule) in &governance.automod_rules {
        if !rule.enabled {
            continue;
        }
        let trigger: TriggerConfig = serde_json::from_str(&rule.trigger_json)
            .map_err(|e| ChannelError::Adapter(format!("invalid automod rule trigger: {e}")))?;
        // Architecture §20.4 + §26 W26 — bound the compiled NFA so an
        // adversarial admin can't ship a regex that DoSes every other
        // peer's first-message-receive. validate_automod_rule already
        // enforces these limits at write time; this is the
        // belt-and-braces enforcement on the receive side.
        let regexes = trigger
            .regex_patterns
            .iter()
            .map(|pattern| {
                regex::RegexBuilder::new(pattern)
                    .size_limit(256 * 1024)
                    .build()
                    .map_err(|e| ChannelError::Adapter(format!("invalid automod regex: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        rules.push(CompiledAutoModRule {
            rule_id: *rule_id,
            name: rule.name.clone(),
            keywords_lower: trigger
                .keywords
                .into_iter()
                .map(|keyword| keyword.trim().to_lowercase())
                .filter(|keyword| !keyword.is_empty())
                .collect(),
            regexes,
            action: rule.action,
        });
    }
    rules.sort_by_key(|rule| rule.rule_id);
    Ok(rules)
}

fn rule_fingerprint(governance: &GovernanceState) -> Vec<([u8; 16], OpStamp)> {
    let mut fingerprint = governance
        .automod_rules
        .iter()
        .map(|(rule_id, rule)| (*rule_id, rule.stamp))
        .collect::<Vec<_>>();
    fingerprint.sort_by(|a, b| a.0.cmp(&b.0));
    fingerprint
}

pub(crate) fn rule_matches(rule: &CompiledAutoModRule, body: &str) -> bool {
    let lower_body = body.to_lowercase();
    rule.keywords_lower
        .iter()
        .any(|keyword| lower_body.contains(keyword))
        || rule.regexes.iter().any(|regex| regex.is_match(body))
}

/// `governance`'s enabled rules, by name then id.
#[must_use]
pub fn rules_of(governance: &GovernanceState) -> Vec<AutoModRuleInfo> {
    let mut rules = governance
        .automod_rules
        .iter()
        .filter(|(_, rule)| rule.enabled)
        .filter_map(|(rule_id, rule)| {
            let trigger = serde_json::from_str::<TriggerConfig>(&rule.trigger_json).ok()?;
            Some(AutoModRuleInfo {
                rule_id: hex::encode(rule_id),
                name: rule.name.clone(),
                enabled: rule.enabled,
                keywords: trigger.keywords,
                regex_patterns: trigger.regex_patterns,
                action: rule.action,
                lamport: rule.stamp.lamport,
            })
        })
        .collect::<Vec<_>>();
    rules.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.rule_id.cmp(&b.rule_id)));
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyword_rule(name: &str, kw: &str, action: AutoModAction) -> CompiledAutoModRule {
        CompiledAutoModRule {
            rule_id: [0u8; 16],
            name: name.into(),
            keywords_lower: vec![kw.into()],
            regexes: vec![],
            action,
        }
    }

    #[test]
    fn rule_matches_lowercase_keyword_in_mixed_case_body() {
        let rule = keyword_rule("test", "foo", AutoModAction::BlockLocally);
        assert!(rule_matches(&rule, "Hello FOO world"));
        assert!(!rule_matches(&rule, "Hello bar world"));
    }

    #[test]
    fn rule_matches_regex() {
        let rule = CompiledAutoModRule {
            rule_id: [0u8; 16],
            name: "regex test".into(),
            keywords_lower: vec![],
            regexes: vec![regex::Regex::new(r"\d{3}-\d{4}").unwrap()],
            action: AutoModAction::BlurContent,
        };
        assert!(rule_matches(&rule, "Call 555-1234 now"));
        assert!(!rule_matches(&rule, "Call me later"));
    }

    #[test]
    fn rule_does_not_match_empty_body() {
        let rule = keyword_rule("test", "foo", AutoModAction::BlockLocally);
        assert!(!rule_matches(&rule, ""));
    }
}
