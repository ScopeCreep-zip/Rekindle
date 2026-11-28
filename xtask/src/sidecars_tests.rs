use super::{conf_problems, entry_problems, installed_name, staged_name, CONFIG_FLAG, HELPER_NAME};
use serde_json::json;

#[test]
fn staged_and_installed_names_follow_tauri() {
    let win = staged_name(HELPER_NAME, "x86_64-pc-windows-msvc");
    assert_eq!(win, "rekindle-skin-helper-x86_64-pc-windows-msvc.exe");
    assert_eq!(
        installed_name(&win, "x86_64-pc-windows-msvc"),
        "rekindle-skin-helper.exe"
    );
    let mac = staged_name(HELPER_NAME, "aarch64-apple-darwin");
    assert_eq!(mac, "rekindle-skin-helper-aarch64-apple-darwin");
    assert_eq!(
        installed_name(&mac, "aarch64-apple-darwin"),
        "rekindle-skin-helper"
    );
}

fn base() -> serde_json::Value {
    json!({"build": {"beforeBuildCommand": "pnpm build"}, "bundle": {}})
}

fn release() -> serde_json::Value {
    json!({
        "build": {"beforeBuildCommand": "pnpm build && cargo xtask stage-sidecar"},
        "bundle": {"externalBin": ["binaries/rekindle-skin-helper"]}
    })
}

#[test]
fn the_configs_must_bundle_and_stage_the_helper() {
    assert!(conf_problems(&base(), &release()).is_empty());
    let mut none = release();
    none["bundle"] = json!({});
    assert_eq!(conf_problems(&base(), &none).len(), 1);
    let mut suffixed = release();
    suffixed["bundle"]["externalBin"] = json!(["binaries/rekindle-skin-helper.exe"]);
    // Missing the exact entry, and it installs as `….exe.exe` on Windows
    // and `….exe` elsewhere: every release triple is a hit.
    assert_eq!(conf_problems(&base(), &suffixed).len(), 5);
    let mut unstaged = release();
    unstaged["build"]["beforeBuildCommand"] = json!("pnpm build");
    assert_eq!(conf_problems(&base(), &unstaged).len(), 1);
    let mut no_frontend = release();
    no_frontend["build"]["beforeBuildCommand"] = json!("cargo xtask stage-sidecar");
    assert_eq!(conf_problems(&base(), &no_frontend).len(), 1);
    let mut in_base = base();
    in_base["bundle"]["externalBin"] = json!(["binaries/rekindle-skin-helper"]);
    assert_eq!(conf_problems(&in_base, &release()).len(), 1);
}

#[test]
fn every_release_entry_point_passes_the_overlay() {
    let yml = format!(
        "      - name: Build\n        uses: tauri-apps/tauri-action@v0\n        with:\n          args: ${{{{ matrix.args }}}} {CONFIG_FLAG}\n"
    );
    let pkg = json!({"scripts": {"bundle": format!("tauri build {CONFIG_FLAG}")}});
    let good = "pnpm bundle\n";
    assert!(entry_problems(&yml, &pkg, good, good).is_empty());
    let bare_yml = yml.replace(CONFIG_FLAG, "");
    assert_eq!(entry_problems(&bare_yml, &pkg, good, good).len(), 1);
    let no_script = json!({"scripts": {}});
    assert_eq!(entry_problems(&yml, &no_script, good, good).len(), 1);
    let bare = "pnpm bundle\npnpm tauri build\n";
    assert_eq!(entry_problems(&yml, &pkg, bare, good).len(), 1);
    assert_eq!(
        entry_problems(&yml, &pkg, good, "pnpm tauri build").len(),
        1
    );
}
