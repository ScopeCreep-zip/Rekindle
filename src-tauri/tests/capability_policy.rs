//! Capability (ACL) policy tests.
//!
//! Tauri's build-time `validate_capabilities` only rejects *unknown*
//! permission identifiers. It cannot tell that a command was never granted
//! to any window (the call fails at runtime with "not allowed by ACL"), that
//! a window was handed a blanket `*:default` bundle, or that the
//! pre-authentication login window gained a post-login command. Those are
//! policy, so they are asserted here against the same files the app ships.
//!
//! The command list comes from `src/invoke.rs` through the same parser
//! `build.rs` uses, so a command added there without a grant fails this
//! test instead of failing in the running app.

#[path = "../build_support/invoke_commands.rs"]
mod invoke_commands;
#[path = "../src/window_labels.rs"]
mod window_labels;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use invoke_commands::InvokeCommand;
use window_labels::WindowKind;

struct Capability {
    file: String,
    windows: Vec<String>,
    permissions: Vec<String>,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_dir(dir: &Path) -> Vec<Capability> {
    let mut out = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();
    for path in entries {
        let raw = std::fs::read_to_string(&path).expect("capability readable");
        let v: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{}: invalid JSON: {e}", path.display()));
        let strings = |key: &str| -> Vec<String> {
            v[key]
                .as_array()
                .unwrap_or_else(|| panic!("{}: `{key}` must be an array", path.display()))
                .iter()
                .map(|s| {
                    s.as_str()
                        .unwrap_or_else(|| {
                            panic!(
                                "{}: `{key}` entries must be strings (no inline scopes)",
                                path.display()
                            )
                        })
                        .to_owned()
                })
                .collect()
        };
        out.push(Capability {
            file: path.file_name().unwrap().to_string_lossy().into_owned(),
            windows: strings("windows"),
            permissions: strings("permissions"),
        });
    }
    out
}

fn shipped() -> Vec<Capability> {
    load_dir(&manifest_dir().join("capabilities"))
}

fn dev() -> Vec<Capability> {
    load_dir(&manifest_dir().join("capabilities-dev"))
}

fn commands() -> Vec<InvokeCommand> {
    let src = std::fs::read_to_string(manifest_dir().join("src/invoke.rs")).expect("invoke.rs");
    invoke_commands::parse(&src).expect("invoke.rs parses")
}

/// The ACL permission identifier `tauri-build` generates for a command.
fn allow_permission(name: &str) -> String {
    format!("allow-{}", name.replace('_', "-"))
}

/// App-command grants are the un-namespaced `allow-*` identifiers;
/// everything with a `:` belongs to core or a plugin.
fn is_app_grant(p: &str) -> bool {
    !p.contains(':')
}

/// Every window label pattern the app can create, from the same
/// `WindowKind` table `src/windows.rs` builds windows from.
fn window_patterns() -> BTreeSet<String> {
    WindowKind::ALL
        .iter()
        .map(|k| k.pattern().to_owned())
        .collect()
}

#[test]
fn no_blanket_or_escalating_grants() {
    for cap in shipped().iter().chain(dev().iter()) {
        for p in &cap.permissions {
            assert!(
                !p.ends_with(":default") && p != "default",
                "{}: `{p}` — plugin/core `default` bundles grant more than the window uses",
                cap.file
            );
            assert!(
                !p.starts_with("core:webview:"),
                "{}: `{p}` — webviews/windows are created only from Rust (S8)",
                cap.file
            );
            assert!(
                !p.starts_with("core:event:"),
                "{}: `{p}` — events arrive on each window's own channel (`subscribe_events`); \
                 the global event plugin would let a window listen to, or forge, every event (S9)",
                cap.file
            );
            assert!(
                !p.starts_with("core:app:")
                    && !p.starts_with("core:path:")
                    && !p.starts_with("core:resources:"),
                "{}: `{p}` — not used by the frontend",
                cap.file
            );
        }
    }
}

#[test]
fn default_capability_is_window_chrome_only() {
    let caps = shipped();
    let default = caps
        .iter()
        .find(|c| c.file == "default.json")
        .expect("default.json");
    for p in &default.permissions {
        assert!(
            p.starts_with("core:window:allow-"),
            "default.json: `{p}` belongs in a per-window app-*.json file"
        );
    }
}

#[test]
fn login_window_is_pre_auth_only() {
    let expected: BTreeSet<String> = [
        "create_identity",
        "delete_identity",
        "get_avatar",
        // The review of the delete it issues (its passphrase proof).
        "intent_cancel",
        "intent_commit",
        "lifecycle_current",
        "list_identities",
        "login",
        "show_buddy_list",
        // The login window's event stream; its audience is lifecycle
        // transitions only (`event_dispatch::WebviewEvent::audience`).
        "subscribe_events",
    ]
    .iter()
    .map(|c| allow_permission(c))
    .collect();
    let caps = shipped();
    let login = caps
        .iter()
        .find(|c| c.file == "app-login.json")
        .expect("app-login.json");
    assert_eq!(login.windows, vec!["login".to_owned()]);
    let got: BTreeSet<String> = login.permissions.iter().cloned().collect();
    assert_eq!(
        got, expected,
        "app-login.json grants changed — the login window runs before any identity is unlocked"
    );
    for cap in caps.iter().chain(dev().iter()) {
        if cap.file != "app-login.json" && cap.file != "default.json" {
            assert!(
                !cap.windows.iter().any(|w| w == "login"),
                "{}: only app-login.json and default.json may target the login window",
                cap.file
            );
        }
    }
}

#[test]
fn capability_windows_match_window_kinds() {
    let patterns = window_patterns();
    let mut covered = BTreeSet::new();
    for cap in shipped().iter().chain(dev().iter()) {
        for w in &cap.windows {
            assert!(
                patterns.contains(w),
                "{}: window `{w}` is not a `WindowKind` pattern ({patterns:?})",
                cap.file
            );
            covered.insert(w.clone());
        }
    }
    assert_eq!(covered, patterns, "every `WindowKind` needs a capability");
}

#[test]
fn every_command_is_granted_where_it_belongs() {
    let caps: Vec<(Capability, bool)> = shipped()
        .into_iter()
        .map(|c| (c, true))
        .chain(dev().into_iter().map(|c| (c, false)))
        .collect();
    let mut grants: BTreeMap<String, Vec<&(Capability, bool)>> = BTreeMap::new();
    for entry in &caps {
        for p in entry.0.permissions.iter().filter(|p| is_app_grant(p)) {
            grants.entry(p.clone()).or_default().push(entry);
        }
    }
    for cmd in commands() {
        let perm = allow_permission(cmd.name());
        let granted = grants.remove(&perm).unwrap_or_default();
        let debug_only = matches!(cmd, InvokeCommand::DebugOnly(_));
        assert!(
            !granted.is_empty(),
            "`{}` is registered in src/invoke.rs but granted to no window",
            cmd.name()
        );
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for (cap, in_shipped) in granted {
            assert_eq!(
                *in_shipped,
                !debug_only,
                "`{}` is granted from {} — debug-only commands belong in capabilities-dev/, release commands in capabilities/",
                cmd.name(),
                cap.file
            );
            for w in &cap.windows {
                if let Some(prev) = seen.insert(w, &cap.file) {
                    panic!(
                        "`{}` is granted to window `{w}` twice ({prev} and {}) — keep one owner per window",
                        cmd.name(), cap.file
                    );
                }
            }
        }
    }
    assert!(
        grants.is_empty(),
        "grants for commands not registered in src/invoke.rs: {:?}",
        grants.keys().collect::<Vec<_>>()
    );
}

/// Every window is built in `src/windows.rs` through one builder that
/// installs the navigation guard. A window declared in `tauri.conf.json`
/// would be created by Tauri before `setup` and bypass it.
#[test]
fn config_declares_no_windows() {
    let raw =
        std::fs::read_to_string(manifest_dir().join("tauri.conf.json")).expect("tauri.conf.json");
    let conf: serde_json::Value = serde_json::from_str(&raw).expect("tauri.conf.json is JSON");
    let windows = conf["app"]["windows"].as_array().map_or(0, Vec::len);
    assert_eq!(windows, 0, "tauri.conf.json `app.windows` must be empty");
}

/// The pre-authentication login window, profile viewers and the buddy list
/// never reach the camera or microphone.
#[test]
fn only_media_windows_capture() {
    for kind in WindowKind::ALL {
        let expected = !matches!(
            kind,
            WindowKind::Login | WindowKind::Profile | WindowKind::BuddyList
        );
        assert_eq!(kind.captures_media(), expected, "{kind:?}");
    }
}

/// The frontend code a window owns: its entry module and, when it has one,
/// its own folder under `src/windows/`. Shared modules (stores, actions,
/// hydration) import nearly every command wrapper, so the import closure
/// cannot tell which commands a window really calls; the code it owns can
/// (plan C7.28).
fn owned_frontend_code(kind: WindowKind) -> (&'static str, Option<&'static str>) {
    match kind {
        WindowKind::Login => ("src/windows/LoginWindow.tsx", None),
        WindowKind::BuddyList => ("src/windows/BuddyListWindow.tsx", None),
        WindowKind::Settings => (
            "src/windows/SettingsWindow.tsx",
            Some("src/windows/settings"),
        ),
        WindowKind::Chat => (
            "src/windows/ChatWindow.tsx",
            Some("src/windows/chat_window"),
        ),
        WindowKind::Community => (
            "src/windows/CommunityWindow.tsx",
            Some("src/windows/community_window"),
        ),
        WindowKind::Call => ("src/windows/CallWindow.tsx", None),
        WindowKind::Profile => ("src/windows/ProfileWindow.tsx", None),
    }
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

fn frontend_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            frontend_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            out.push(path);
        }
    }
}

/// `commands.<wrapper>` → the command it invokes, from `src/ipc/commands/`.
fn command_wrappers() -> BTreeMap<String, String> {
    let invoke = regex::Regex::new(r#"invoke\b[^"\n]*?\(\s*"(\w+)""#).expect("regex");
    let key = regex::Regex::new(r"(?m)^\s*(\w+)\s*[:(]").expect("regex");
    let mut files = Vec::new();
    frontend_files(&repo_root().join("src/ipc/commands"), &mut files);
    let mut map = BTreeMap::new();
    for file in files {
        let src = std::fs::read_to_string(&file).expect("readable");
        for m in invoke.captures_iter(&src) {
            let at = m.get(0).expect("match").start();
            if let Some(k) = key.captures_iter(&src[..at]).last() {
                map.insert(k[1].to_owned(), m[1].to_owned());
            }
        }
    }
    map
}

/// Every command a window's own code calls is granted to that window. A
/// call it is not granted fails at runtime with "not allowed by ACL", and
/// a `.catch` can hide that: the Settings camera list read the denial of
/// `native_video_capture_available` as "no native capture" (plan C7.28).
#[test]
fn windows_are_granted_the_commands_their_own_code_calls() {
    let wrappers = command_wrappers();
    assert!(
        wrappers.len() > 100,
        "found only {} command wrappers",
        wrappers.len()
    );
    let call = regex::Regex::new(r"\bcommands\s*\.\s*(\w+)\s*\(").expect("regex");
    let caps = shipped();
    for kind in WindowKind::ALL {
        let (entry, dir) = owned_frontend_code(kind);
        let mut files = vec![repo_root().join(entry)];
        if let Some(dir) = dir {
            frontend_files(&repo_root().join(dir), &mut files);
        }
        let granted: BTreeSet<String> = caps
            .iter()
            .filter(|c| c.windows.iter().any(|w| w == kind.pattern()))
            .flat_map(|c| c.permissions.iter().cloned())
            .collect();
        for file in &files {
            let src =
                std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            for m in call.captures_iter(&src) {
                let Some(cmd) = wrappers.get(&m[1]) else {
                    continue;
                };
                assert!(
                    granted.contains(&allow_permission(cmd)),
                    "{} calls `{cmd}`, which no capability grants to window `{}`",
                    file.display(),
                    kind.pattern()
                );
            }
        }
    }
}
