#[test]
fn under_lays_every_directory_out_below_the_base() {
    let base = std::path::Path::new("/tmp/walk-root");
    let root = super::DataRoot::under(base);
    for dir in [&root.data, &root.config, &root.logs, &root.state] {
        assert!(dir.starts_with(base), "{}", dir.display());
    }
    assert_eq!(root.state, base.join("data/state"));
    assert_ne!(root.data, root.config);
}

/// A wipe on a root under a base (the walk's) removes only webview
/// stores below that base, never a real install's (Linux finding F1).
#[test]
fn a_contained_root_wipes_only_webview_stores_below_its_base() {
    let base = std::path::Path::new("/tmp/walk-root");
    let dirs = super::DataRoot::under(base).webview.dirs();
    assert!(!dirs.is_empty());
    for dir in &dirs {
        assert!(dir.starts_with(base), "{}", dir.display());
    }
}

use super::*;

/// The desktop's directories are named after `tauri.conf.json`'s
/// identifier; the hosts that share them must use the same name.
#[test]
fn identifier_matches_tauri_conf() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], IDENTIFIER);
}

#[test]
fn layout_follows_the_tauri_rules() {
    let root = DataRoot::resolve().unwrap();
    assert_eq!(root.data, dirs::data_dir().unwrap().join(IDENTIFIER));
    assert_eq!(root.config, dirs::config_dir().unwrap().join(IDENTIFIER));
    assert_eq!(root.state, root.data.join("state"));
    assert_eq!(root.identity_files("pk"), root.state.join("pk.files"));
    assert!(matches!(root.webview, WebviewStore::Platform(_)));
    assert!(root.logs.ends_with(if cfg!(target_os = "macos") {
        "Library/Logs/com.rekindle.app"
    } else {
        "com.rekindle.app/logs"
    }));
}
