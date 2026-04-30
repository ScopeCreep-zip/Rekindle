use std::sync::Arc;

use regex::Regex;
use serde::Deserialize;

use crate::state::{AppState, AutoModCompiledCache, CompiledAutoModRule};

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

pub fn list_rules(state: &Arc<AppState>, community_id: &str) -> Result<Vec<AutoModRuleInfo>, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let Some(governance) = community.governance_state.as_ref() else {
        return Ok(Vec::new());
    };

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
    Ok(rules)
}

pub fn evaluate_message(
    state: &Arc<AppState>,
    community_id: &str,
    body: &str,
) -> Result<AutoModAction, String> {
    let compiled = compiled_rules(state, community_id)?;
    if compiled.iter().any(|rule| rule_matches(rule, body) && rule.action == "block_locally") {
        return Ok(AutoModAction::BlockLocally);
    }
    if compiled.iter().any(|rule| rule_matches(rule, body) && rule.action == "blur_content") {
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

pub fn get_rule(
    state: &Arc<AppState>,
    community_id: &str,
    rule_id: &[u8; 16],
) -> Result<AutoModRuleInfo, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let governance = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded")?;
    let rule = governance
        .automod_rules
        .get(rule_id)
        .ok_or("automod rule not found")?;
    let trigger: TriggerConfig = serde_json::from_str(&rule.trigger_json)
        .map_err(|e| format!("invalid automod rule trigger: {e}"))?;
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

fn compiled_rules(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<Vec<CompiledAutoModRule>, String> {
    let fingerprint = rule_fingerprint(state, community_id)?;
    {
        let cache = state.automod_cache.read();
        if let Some(compiled) = cache.get(community_id) {
            if compiled.fingerprint == fingerprint {
                return Ok(compiled.rules.clone());
            }
        }
    }

    let compiled = compile_rules(state, community_id, &fingerprint)?;
    let rules = compiled.rules.clone();
    state
        .automod_cache
        .write()
        .insert(community_id.to_string(), compiled);
    Ok(rules)
}

fn compile_rules(
    state: &Arc<AppState>,
    community_id: &str,
    fingerprint: &[([u8; 16], u64)],
) -> Result<AutoModCompiledCache, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let governance = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded")?;

    let mut rules = Vec::new();
    for (rule_id, rule) in &governance.automod_rules {
        if !rule.enabled {
            continue;
        }
        let trigger: TriggerConfig =
            serde_json::from_str(&rule.trigger_json).map_err(|e| format!("invalid automod rule trigger: {e}"))?;
        let regexes = trigger
            .regex_patterns
            .iter()
            .map(|pattern| Regex::new(pattern).map_err(|e| format!("invalid automod regex: {e}")))
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

fn rule_fingerprint(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<Vec<([u8; 16], u64)>, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let governance = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded")?;
    let mut fingerprint = governance
        .automod_rules
        .iter()
        .map(|(rule_id, rule)| (*rule_id, rule.lamport))
        .collect::<Vec<_>>();
    fingerprint.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(fingerprint)
}

fn rule_matches(rule: &CompiledAutoModRule, body: &str) -> bool {
    let lower_body = body.to_lowercase();
    rule.keywords_lower
        .iter()
        .any(|keyword| lower_body.contains(keyword))
        || rule.regexes.iter().any(|regex| regex.is_match(body))
}
