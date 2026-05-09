//! Windows keyring platform support.
//!
//! The `keyring` crate v3 on Windows uses the Credential Manager (DPAPI).
//! This is available on all Windows versions supported by Rust.

pub const BACKEND_NAME: &str = "Windows Credential Manager (DPAPI)";

/// Windows-specific diagnostics for `rekindle status --doctor`.
pub fn diagnostics() -> Vec<(String, String)> {
    vec![("backend".into(), BACKEND_NAME.into())]
}
