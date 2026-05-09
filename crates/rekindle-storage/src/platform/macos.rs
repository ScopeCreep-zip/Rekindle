//! macOS keyring platform support.
//!
//! The `keyring` crate v3 on macOS uses the Security Framework (Keychain).
//! This is available on all macOS versions supported by Rust.

pub const BACKEND_NAME: &str = "macOS Keychain (Security Framework)";

/// macOS-specific diagnostics for `rekindle status --doctor`.
pub fn diagnostics() -> Vec<(String, String)> {
    vec![("backend".into(), BACKEND_NAME.into())]
}
