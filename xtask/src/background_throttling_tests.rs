use super::*;

const GOOD: &str = r"
        fn base_builder() {
            let b = WebviewWindowBuilder::new(app, label, url)
                .background_throttling(BackgroundThrottlingPolicy::Throttle);
        }
        fn open() { base_builder(); }
    ";

#[test]
fn base_builder_with_throttle_passes() {
    assert!(source_problems(WINDOWS_RS, GOOD).unwrap().is_empty());
}

#[test]
fn missing_or_other_policy_fails() {
    let none = GOOD.replace(
        ".background_throttling(BackgroundThrottlingPolicy::Throttle)",
        "",
    );
    assert_eq!(source_problems(WINDOWS_RS, &none).unwrap().len(), 1);
    let suspend = GOOD.replace("::Throttle", "::Suspend");
    assert_eq!(source_problems(WINDOWS_RS, &suspend).unwrap().len(), 1);
}

const BY_KIND: &str = r"
        const fn background_policy(kind: WindowKind) -> BackgroundThrottlingPolicy {
            match kind {
                WindowKind::Call => BackgroundThrottlingPolicy::Disabled,
                _ => BackgroundThrottlingPolicy::Throttle,
            }
        }
        fn base_builder() {
            let b = WebviewWindowBuilder::new(app, label, url)
                .background_throttling(background_policy(kind));
        }
    ";

#[test]
fn a_policy_by_kind_of_throttle_and_disabled_passes() {
    assert!(source_problems(WINDOWS_RS, BY_KIND).unwrap().is_empty());
}

#[test]
fn a_policy_by_kind_that_suspends_or_never_throttles_fails() {
    let suspend = BY_KIND.replace("::Disabled", "::Suspend");
    assert_eq!(source_problems(WINDOWS_RS, &suspend).unwrap().len(), 1);
    let never = BY_KIND.replace("::Throttle", "::Disabled");
    assert_eq!(source_problems(WINDOWS_RS, &never).unwrap().len(), 1);
    let undefined = BY_KIND.replace("const fn background_policy", "const fn other_policy");
    assert_eq!(source_problems(WINDOWS_RS, &undefined).unwrap().len(), 1);
}

#[test]
fn builder_outside_base_builder_fails() {
    let other = "fn open() { let b = tauri::WebviewWindowBuilder::from_config(app, &c); }";
    assert_eq!(
        source_problems("src-tauri/src/x.rs", other).unwrap().len(),
        1
    );
    let in_mod = format!("{GOOD} fn rogue() {{ WindowBuilder::new(app, l); }}");
    assert_eq!(source_problems(WINDOWS_RS, &in_mod).unwrap().len(), 1);
}

#[test]
fn conf_windows_need_throttle() {
    let empty = serde_json::json!({"app": {"windows": []}});
    assert!(conf_problems(&empty).is_empty());
    let bad = serde_json::json!({"app": {"windows": [{"label": "x"}]}});
    assert_eq!(conf_problems(&bad).len(), 1);
    let good = serde_json::json!({"app": {"windows": [
        {"label": "x", "backgroundThrottling": "throttle"}
    ]}});
    assert!(conf_problems(&good).is_empty());
}
