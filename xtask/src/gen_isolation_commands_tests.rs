use super::*;

#[test]
fn a_core_grant_names_its_plugin_command() {
    assert_eq!(
        plugin_command("core:window:allow-toggle-maximize")
            .unwrap()
            .as_deref(),
        Some("plugin:window|toggle_maximize")
    );
    assert_eq!(
        plugin_command("deep-link:allow-get-current")
            .unwrap()
            .as_deref(),
        Some("plugin:deep-link|get_current")
    );
    assert_eq!(plugin_command("allow-get-friends").unwrap(), None);
}

#[test]
fn a_bundle_grant_is_refused() {
    assert!(plugin_command("core:default").is_err());
    assert!(plugin_command("core:window:deny-close").is_err());
}

#[test]
fn the_repo_list_holds_app_plugin_and_channel_commands_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let list = commands(root).unwrap();
    assert!(list.contains("login"));
    assert!(list.contains("plugin:window|start_dragging"));
    assert!(list.contains(CHANNEL_FETCH));
    assert!(!list.contains("plugin:webview|internal_toggle_devtools"));
    assert!(!list.iter().any(|c| c.starts_with("plugin:event|")));
}
