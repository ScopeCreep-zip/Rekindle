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
