//! Automod rule validation: trigger JSON shape, regex-DoS guards, and
//! workspace-wide keyword/regex quotas.

use crate::state::GovernanceState;

const MAX_AUTOMOD_KEYWORDS: usize = 1000;
const MAX_AUTOMOD_REGEX_PATTERNS: usize = 10;
/// Architecture §20.4 + §26 W26 — adversarial admins can ship pathological
/// regex patterns whose compiled NFA fits in the default 10 MiB ceiling
/// but still takes seconds to assemble. Cap individual pattern source
/// length so a malicious `AutoModRule` can't DoS every other peer's
/// `compile_rules` pass on first message receive.
const MAX_AUTOMOD_REGEX_PATTERN_LEN: usize = 512;
const MAX_AUTOMOD_KEYWORD_LEN: usize = 256;
/// Tighten the regex compiler's NFA budget below the 10 MiB default.
/// Architecture §20.4 — moderation rules are advisory and short by
/// design; 256 KiB compiled NFA is plenty for the stated keyword/regex
/// patterns and rejects anything pathological well before it becomes a
/// receive-side DoS.
const AUTOMOD_REGEX_SIZE_LIMIT: usize = 256 * 1024;

pub(super) fn validate_automod_rule(
    rule_id: &[u8; 16],
    enabled: bool,
    trigger_json: &str,
    action: &str,
    state: &GovernanceState,
) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TriggerConfig {
        #[serde(default)]
        keywords: Vec<String>,
        #[serde(default)]
        regex_patterns: Vec<String>,
    }

    if !matches!(
        action,
        "block_locally" | "blur_content" | "alert_moderators"
    ) {
        return false;
    }

    let Ok(trigger) = serde_json::from_str::<TriggerConfig>(trigger_json) else {
        return false;
    };
    // Length caps before compile so a 1 MB regex source can't even
    // reach the regex parser. Mirrors the spec's "small filter rules"
    // intent (§20.4) and prevents per-rule receive-side DoS.
    if trigger
        .keywords
        .iter()
        .any(|k| k.len() > MAX_AUTOMOD_KEYWORD_LEN)
    {
        return false;
    }
    if trigger
        .regex_patterns
        .iter()
        .any(|pattern| pattern.len() > MAX_AUTOMOD_REGEX_PATTERN_LEN)
    {
        return false;
    }
    if trigger.regex_patterns.iter().any(|pattern| {
        regex::RegexBuilder::new(pattern)
            .size_limit(AUTOMOD_REGEX_SIZE_LIMIT)
            .build()
            .is_err()
    }) {
        return false;
    }

    let current_totals = state
        .automod_rules
        .iter()
        .filter(|(existing_id, rule)| *existing_id != rule_id && rule.enabled)
        .filter_map(|(_, rule)| serde_json::from_str::<TriggerConfig>(&rule.trigger_json).ok())
        .fold((0usize, 0usize), |(keywords, regexes), trigger| {
            (
                keywords + trigger.keywords.len(),
                regexes + trigger.regex_patterns.len(),
            )
        });

    let next_keywords = current_totals.0 + if enabled { trigger.keywords.len() } else { 0 };
    let next_regexes = current_totals.1
        + if enabled {
            trigger.regex_patterns.len()
        } else {
            0
        };
    next_keywords <= MAX_AUTOMOD_KEYWORDS && next_regexes <= MAX_AUTOMOD_REGEX_PATTERNS
}
