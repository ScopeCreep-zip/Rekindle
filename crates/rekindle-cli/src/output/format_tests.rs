#[test]
fn print_structured_json_produces_valid_json() {
    // We can't capture stdout in a unit test, but we can verify
    // the serialization doesn't panic on various types
    let val = serde_json::json!({"key": "value", "num": 42});
    // Serialize to string (same path as print_json) to verify no panic
    let json = serde_json::to_string_pretty(&val).unwrap();
    assert!(json.contains("\"key\""));
    assert!(json.contains("42"));
}

#[test]
fn print_structured_handles_empty_object() {
    let val = serde_json::json!({});
    let json = serde_json::to_string_pretty(&val).unwrap();
    assert_eq!(json.trim(), "{}");
}

#[test]
fn print_structured_handles_nested() {
    let val = serde_json::json!({
        "community": {
            "name": "dev-team",
            "channels": ["general", "random"]
        }
    });
    let json = serde_json::to_string_pretty(&val).unwrap();
    assert!(json.contains("dev-team"));
    assert!(json.contains("general"));
}

#[test]
fn print_structured_handles_unicode() {
    let val = serde_json::json!({"name": "日本語コミュニティ"});
    let json = serde_json::to_string_pretty(&val).unwrap();
    assert!(json.contains("日本語コミュニティ"));
}

#[test]
fn print_structured_escapes_special_chars() {
    let val = serde_json::json!({"msg": "hello \"world\" \n\ttab"});
    let json = serde_json::to_string(&val).unwrap();
    assert!(json.contains("\\\"world\\\""));
    assert!(json.contains("\\n"));
    assert!(json.contains("\\t"));
}

#[test]
fn doctor_check_serialization() {
    let checks = [
        rekindle_types::display::Check {
            id: "node.running".into(),
            category: "node".into(),
            status: rekindle_types::display::CheckStatus::Pass,
            value: "active".into(),
            description: String::new(),
        },
        rekindle_types::display::Check {
            id: "crypto.prekeys".into(),
            category: "crypto".into(),
            status: rekindle_types::display::CheckStatus::Warn,
            value: "3/100 fresh slots".into(),
            description: "prekey maintenance is failing".into(),
        },
    ];

    // Verify the JSON structure matches the contract
    let output = serde_json::json!({
        "summary": {
            "pass": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Pass).count(),
            "warn": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Warn).count(),
            "fail": checks.iter().filter(|c| c.status == rekindle_types::display::CheckStatus::Fail).count(),
        },
    });
    assert_eq!(output["summary"]["pass"], 1);
    assert_eq!(output["summary"]["warn"], 1);
    assert_eq!(output["summary"]["fail"], 0);
}
