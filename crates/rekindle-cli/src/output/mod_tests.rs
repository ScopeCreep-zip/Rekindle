use super::*;

#[test]
fn detect_json_from_flag() {
    assert_eq!(OutputMode::detect(Some("json"), false), OutputMode::Json);
}

#[test]
fn detect_jsonl_from_flag() {
    assert_eq!(OutputMode::detect(Some("jsonl"), false), OutputMode::Jsonl);
}

#[test]
fn detect_text_from_flag() {
    assert_eq!(OutputMode::detect(Some("text"), false), OutputMode::Text);
}

#[test]
fn detect_text_default() {
    assert_eq!(OutputMode::detect(None, false), OutputMode::Text);
}

#[test]
fn detect_script_forces_jsonl() {
    assert_eq!(OutputMode::detect(None, true), OutputMode::Jsonl);
}

#[test]
fn format_flag_overrides_script() {
    // --format json wins over --script
    assert_eq!(OutputMode::detect(Some("json"), true), OutputMode::Json);
}

#[test]
fn json_never_uses_color() {
    assert!(!OutputMode::Json.use_color());
    assert!(!OutputMode::Jsonl.use_color());
}

#[test]
fn structured_check() {
    assert!(OutputMode::Json.is_structured());
    assert!(OutputMode::Jsonl.is_structured());
    assert!(!OutputMode::Text.is_structured());
}
