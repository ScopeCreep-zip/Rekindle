//! Content-Security-Policy regression tests.
//!
//! The CSP in `tauri.conf.json` (`app.security.csp`) is the load-bearing
//! mitigation against XSS: SolidJS escaping stops injected markup from
//! becoming live DOM, and the CSP is the backstop for anything that
//! slips past. Weakening a directive here is a security change and
//! should fail CI, not pass review silently.
//!
//! These assertions live in Rust because the policy is Rust-side
//! configuration: Tauri injects it via its custom protocol, so only the
//! built app carries it. `cargo test` reads the same file the app ships
//! with, on every CI run, with no browser or build step.
//!
//! Runtime *enforcement* (does the WebView actually refuse an inline
//! script?) is a different question and is not covered here: plan E7.14
//! S6 checks it in the real app (the real-app suite, e2e/app).

use std::path::Path;

/// The shipped Tauri config.
fn config() -> serde_json::Value {
    let raw =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .expect("tauri.conf.json must be readable");
    serde_json::from_str(&raw).expect("tauri.conf.json must be valid JSON")
}

/// Parse `app.security.csp` out of the shipped Tauri config.
fn csp() -> String {
    config()["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp must be a string — an absent CSP is a security regression")
        .to_string()
}

/// Split the policy into `directive -> value` pairs.
fn directive(policy: &str, name: &str) -> Option<String> {
    policy.split(';').map(str::trim).find_map(|part| {
        let rest = part.strip_prefix(name)?;
        // Guard against `script-src` matching `script-src-elem`.
        if rest.starts_with([' ', '\t']) || rest.is_empty() {
            Some(rest.trim().to_string())
        } else {
            None
        }
    })
}

#[test]
fn csp_is_declared() {
    assert!(!csp().trim().is_empty(), "CSP must not be empty");
}

#[test]
fn csp_forbids_unsafe_eval() {
    let policy = csp();
    assert!(
        !policy.contains("'unsafe-eval'"),
        "CSP must never allow 'unsafe-eval': {policy}"
    );
}

#[test]
fn script_src_forbids_unsafe_inline_and_eval() {
    let policy = csp();
    let script_src = directive(&policy, "script-src")
        .or_else(|| directive(&policy, "default-src"))
        .expect("script-src (or a default-src fallback) must be declared");
    assert!(
        !script_src.contains("'unsafe-inline'"),
        "script-src must not allow 'unsafe-inline': {script_src}"
    );
    assert!(
        !script_src.contains("'unsafe-eval'"),
        "script-src must not allow 'unsafe-eval': {script_src}"
    );
}

#[test]
fn object_src_is_none() {
    let policy = csp();
    assert_eq!(
        directive(&policy, "object-src").as_deref(),
        Some("'none'"),
        "object-src must be 'none' — plugins are a bypass vector: {policy}"
    );
}

#[test]
fn frame_ancestors_is_none() {
    let policy = csp();
    assert_eq!(
        directive(&policy, "frame-ancestors").as_deref(),
        Some("'none'"),
        "frame-ancestors must be 'none' to prevent clickjacking: {policy}"
    );
}

/// No plaintext-HTTP origin may appear in `connect-src`. Tauri's own
/// `ipc:`/`http://ipc.localhost` transport and the `skin://` scheme's
/// Windows origin `http://skin.localhost` (plan E7.7.2) are the documented
/// exceptions: both are served in-process and never leave the machine.
#[test]
fn connect_src_has_no_remote_plaintext_origin() {
    let policy = csp();
    let connect_src = directive(&policy, "connect-src")
        .or_else(|| directive(&policy, "default-src"))
        .expect("connect-src (or a default-src fallback) must be declared");

    const LOCAL_IPC_ALLOWED: &[&str] = &["http://ipc.localhost", "http://skin.localhost"];

    for token in connect_src.split_whitespace() {
        assert!(
            !token.starts_with("http://") || LOCAL_IPC_ALLOWED.contains(&token),
            "connect-src allows a remote plaintext origin `{token}`: {connect_src}"
        );
    }
}

/// Images come from the bundle, from backend-built `data:` URLs (avatars,
/// expressions) and from the `skin://` scheme (plan E7.7.2: `skin:` on
/// macOS and Linux, `http://skin.localhost` on Windows). The scheme serves
/// only the Host's validated skin store by content address, so both
/// reasons this pin exists still hold: no remote origin (rendering a
/// peer-chosen image URL would leak every reader's IP to its host) and no
/// `asset:` protocol (it is not enabled; it would expose the filesystem).
#[test]
fn img_src_has_no_asset_or_remote() {
    let policy = csp();
    let img_src = directive(&policy, "img-src").expect("img-src must be declared");
    assert_eq!(
        img_src, "'self' data: skin: http://skin.localhost",
        "img-src widened: {policy}"
    );
}

/// The skin stylesheet loads from the `skin://` scheme and nowhere else
/// remote (plan E7.7.2); `connect-src` names the same two origins.
#[test]
fn style_and_connect_src_name_the_skin_scheme() {
    let policy = csp();
    let style_src = directive(&policy, "style-src").expect("style-src must be declared");
    assert_eq!(
        style_src, "'self' 'unsafe-inline' skin: http://skin.localhost",
        "style-src changed: {policy}"
    );
    let connect_src = directive(&policy, "connect-src").expect("connect-src must be declared");
    assert_eq!(
        connect_src, "ipc: http://ipc.localhost skin: http://skin.localhost",
        "connect-src changed: {policy}"
    );
    assert!(!policy.contains("asset:"), "no asset: protocol: {policy}");
}

/// Audio plays from the bundle, from voice-message `blob:` URLs and from
/// backend-built expression `data:` URLs; nothing remote.
#[test]
fn media_src_is_local_only() {
    let policy = csp();
    let media_src = directive(&policy, "media-src").expect("media-src must be declared");
    assert_eq!(
        media_src, "'self' blob: data:",
        "media-src widened: {policy}"
    );
}

/// `Object.prototype` is frozen before any app script runs, so a
/// prototype-pollution gadget cannot plant properties every object
/// inherits.
#[test]
fn freeze_prototype_enabled() {
    assert_eq!(
        config()["app"]["security"]["freezePrototype"].as_bool(),
        Some(true),
        "app.security.freezePrototype must be true"
    );
}

/// Tauri's isolation pattern (plan E7.1, roadmap 3.3): every IPC message
/// passes `dist-isolation/`'s `__TAURI_ISOLATION_HOOK__` before Rust sees
/// it, and the hook allows only the generated command list (`cargo xtask
/// check-isolation-commands` keeps it current).
#[test]
fn isolation_pattern_is_on() {
    let pattern = &config()["app"]["security"]["pattern"];
    assert_eq!(pattern["use"].as_str(), Some("isolation"), "{pattern}");
    assert_eq!(
        pattern["options"]["dir"].as_str(),
        Some("../dist-isolation"),
        "{pattern}"
    );
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist-isolation");
    let hook = std::fs::read_to_string(dir.join("index.js")).expect("dist-isolation/index.js");
    assert!(hook.contains("window.__TAURI_ISOLATION_HOOK__ ="));
    assert!(hook.contains("REKINDLE_ISOLATION_COMMANDS.includes(cmd)"));
    assert!(dir.join("commands.js").is_file());
}

/// The isolation iframe loads from a scheme Tauri names at build time
/// (`isolation-<uuid>:`, or `http://isolation-<uuid>.localhost` on
/// Windows), and Tauri adds it to `default-src` only (`tauri` 2.12.1
/// `src/manager/mod.rs:430-437`). A `frame-src` would win over that and block
/// the iframe, so frames fall back to `default-src`, which allows `'self'`
/// and nothing remote.
#[test]
fn frames_fall_back_to_a_self_only_default_src() {
    let policy = csp();
    assert_eq!(directive(&policy, "frame-src"), None, "{policy}");
    assert_eq!(directive(&policy, "child-src"), None, "{policy}");
    assert_eq!(
        directive(&policy, "default-src").as_deref(),
        Some("'self'"),
        "{policy}"
    );
}
