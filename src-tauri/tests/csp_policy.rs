//! Content-Security-Policy regression tests.
//!
//! The CSP in `tauri.conf.json` (`app.security.csp`) is the load-bearing
//! mitigation against XSS: SolidJS escaping stops injected markup from
//! becoming live DOM, and the CSP is the backstop for anything that
//! slips past. Weakening a directive here is a security change and
//! should fail CI, not pass review silently.
//!
//! These assertions live in Rust rather than in `e2e/security/csp.spec.ts`
//! because the policy is Rust-side configuration. Tauri injects it via
//! its custom protocol, so it is simply absent when Playwright runs
//! against the Vite dev server — the browser-side tests could never see
//! it. `cargo test` reads the same file the app ships with, on every CI
//! run, with no browser or build step.
//!
//! Runtime *enforcement* (does the WebView actually refuse an inline
//! script?) is a different question and is not covered here — see the
//! skipped tests in `e2e/security/csp.spec.ts`.

use std::path::Path;

/// Parse `app.security.csp` out of the shipped Tauri config.
fn csp() -> String {
    let raw =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .expect("tauri.conf.json must be readable");
    let cfg: serde_json::Value =
        serde_json::from_str(&raw).expect("tauri.conf.json must be valid JSON");
    cfg["app"]["security"]["csp"]
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
/// `ipc:`/`http://ipc.localhost` transport is the documented exception —
/// it never leaves the machine.
#[test]
fn connect_src_has_no_remote_plaintext_origin() {
    let policy = csp();
    let connect_src = directive(&policy, "connect-src")
        .or_else(|| directive(&policy, "default-src"))
        .expect("connect-src (or a default-src fallback) must be declared");

    const LOCAL_IPC_ALLOWED: &[&str] = &["http://ipc.localhost", "http://asset.localhost"];

    for token in connect_src.split_whitespace() {
        assert!(
            !token.starts_with("http://") || LOCAL_IPC_ALLOWED.contains(&token),
            "connect-src allows a remote plaintext origin `{token}`: {connect_src}"
        );
    }
}
