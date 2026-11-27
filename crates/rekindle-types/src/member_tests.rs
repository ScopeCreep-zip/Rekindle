use super::*;

#[test]
fn member_info_roundtrip() {
    let m = MemberInfo {
        pseudonym_key: "abcd".into(),
        display_name: "Alice".into(),
        role_ids: vec!["00".repeat(16), "01".repeat(16)],
        status: "online".into(),
        route_blob: Some(vec![1, 2, 3]),
        bio: Some("hi".into()),
        pronouns: None,
        theme_color: Some(0xff_ee_dd),
        badges: vec!["founder".into()],
        last_seen: 1_700_000_000,
    };
    let json = serde_json::to_string(&m).unwrap();
    let back: MemberInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(m, back);
}

#[test]
fn member_info_defaults_omit_optional() {
    let m = MemberInfo {
        pseudonym_key: "abcd".into(),
        display_name: "Alice".into(),
        role_ids: vec![],
        status: "online".into(),
        route_blob: None,
        bio: None,
        pronouns: None,
        theme_color: None,
        badges: vec![],
        last_seen: 0,
    };
    let json = serde_json::to_string(&m).unwrap();
    // Optional + empty fields are skipped on the wire.
    assert!(!json.contains("timeoutUntil"));
    assert!(!json.contains("routeBlob"));
    assert!(!json.contains("badges"));
}
