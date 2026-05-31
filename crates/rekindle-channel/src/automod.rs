//! Phase 23.D.6 — automod compile + evaluate logic ported from
//! `src-tauri/services/community/automod.rs`. Pure protocol logic that
//! reads governance.automod_rules and matches incoming channel-message
//! bodies against compiled regexes + lowercased keywords. The compiled
//! cache (regex JIT state) lives on the deps adapter; this module
//! provides the rebuild-on-fingerprint-mismatch shape.

use std::sync::Arc;

use regex::Regex;
use rekindle_governance::state::GovernanceState;
use serde::Deserialize;

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// Action emitted by `evaluate_message` for a matching rule. Caller
/// chooses how to apply (drop, blur, or alert moderators).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoModAction {
    Allow,
    BlockLocally,
    BlurContent,
    AlertModerators,
}

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
    pub action: String,
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
    pub action: String,
}

/// Cache snapshot — `fingerprint` is the (rule_id, lamport) tuple set
/// at compile time; rebuild trigger is fingerprint mismatch.
#[derive(Debug, Clone)]
pub struct AutoModCompiledCache {
    pub fingerprint: Vec<([u8; 16], u64)>,
    pub rules: Vec<CompiledAutoModRule>,
}

pub fn list_rules<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<AutoModRuleInfo>, ChannelError> {
    let governance = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    Ok(rules_from_governance(&governance))
}

pub fn evaluate_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    body: &str,
) -> Result<AutoModAction, ChannelError> {
    let compiled = compiled_rules(deps, community_id)?;
    if compiled
        .iter()
        .any(|rule| rule_matches(rule, body) && rule.action == "block_locally")
    {
        return Ok(AutoModAction::BlockLocally);
    }
    if compiled
        .iter()
        .any(|rule| rule_matches(rule, body) && rule.action == "blur_content")
    {
        return Ok(AutoModAction::BlurContent);
    }
    if compiled
        .iter()
        .any(|rule| rule_matches(rule, body) && rule.action == "alert_moderators")
    {
        return Ok(AutoModAction::AlertModerators);
    }
    Ok(AutoModAction::Allow)
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
        action: rule.action.clone(),
        lamport: rule.lamport,
    })
}

fn compiled_rules<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<CompiledAutoModRule>, ChannelError> {
    let governance = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let fingerprint = rule_fingerprint(&governance);

    if let Some(compiled) = deps.automod_compiled_cache_get(community_id) {
        if compiled.fingerprint == fingerprint {
            return Ok(compiled.rules.clone());
        }
    }

    let compiled = compile_rules(&governance, &fingerprint)?;
    let rules = compiled.rules.clone();
    deps.automod_compiled_cache_set(community_id, Arc::new(compiled));
    Ok(rules)
}

fn compile_rules(
    governance: &GovernanceState,
    fingerprint: &[([u8; 16], u64)],
) -> Result<AutoModCompiledCache, ChannelError> {
    let mut rules = Vec::new();
    for (rule_id, rule) in &governance.automod_rules {
        if !rule.enabled {
            continue;
        }
        let trigger: TriggerConfig = serde_json::from_str(&rule.trigger_json).map_err(|e| {
            ChannelError::Adapter(format!("invalid automod rule trigger: {e}"))
        })?;
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
            action: rule.action.clone(),
        });
    }
    Ok(AutoModCompiledCache {
        fingerprint: fingerprint.to_vec(),
        rules,
    })
}

fn rule_fingerprint(governance: &GovernanceState) -> Vec<([u8; 16], u64)> {
    let mut fingerprint = governance
        .automod_rules
        .iter()
        .map(|(rule_id, rule)| (*rule_id, rule.lamport))
        .collect::<Vec<_>>();
    fingerprint.sort_by(|a, b| a.0.cmp(&b.0));
    fingerprint
}

fn rule_matches(rule: &CompiledAutoModRule, body: &str) -> bool {
    let lower_body = body.to_lowercase();
    rule.keywords_lower
        .iter()
        .any(|keyword| lower_body.contains(keyword))
        || rule.regexes.iter().any(|regex| regex.is_match(body))
}

fn rules_from_governance(governance: &GovernanceState) -> Vec<AutoModRuleInfo> {
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
                action: rule.action.clone(),
                lamport: rule.lamport,
            })
        })
        .collect::<Vec<_>>();
    rules.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.rule_id.cmp(&b.rule_id)));
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyword_rule(name: &str, kw: &str, action: &str) -> CompiledAutoModRule {
        CompiledAutoModRule {
            rule_id: [0u8; 16],
            name: name.into(),
            keywords_lower: vec![kw.into()],
            regexes: vec![],
            action: action.into(),
        }
    }

    #[test]
    fn rule_matches_lowercase_keyword_in_mixed_case_body() {
        let rule = keyword_rule("test", "foo", "block_locally");
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
            action: "blur_content".into(),
        };
        assert!(rule_matches(&rule, "Call 555-1234 now"));
        assert!(!rule_matches(&rule, "Call me later"));
    }

    #[test]
    fn rule_does_not_match_empty_body() {
        let rule = keyword_rule("test", "foo", "block_locally");
        assert!(!rule_matches(&rule, ""));
    }
}
