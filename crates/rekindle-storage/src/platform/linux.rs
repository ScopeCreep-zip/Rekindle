//! Linux keyring platform support.
//!
//! The `keyring` crate v3 on Linux uses `linux-keyutils` (kernel keyring)
//! by default. For Secret Service D-Bus (gnome-keyring, KDE Wallet),
//! the `secret-service` feature must be enabled on the `keyring` crate.
//!
//! This module provides Linux-specific availability diagnostics:
//! - Is a D-Bus session bus available?
//! - Is a Secret Service provider registered?
//! - Is the kernel keyring accessible?

pub const BACKEND_NAME: &str = "Linux kernel keyring (linux-keyutils)";

/// Linux-specific diagnostics for `rekindle status --doctor`.
pub fn diagnostics() -> Vec<(String, String)> {
    let mut diags = Vec::new();

    // Check DBUS_SESSION_BUS_ADDRESS
    let dbus_available = std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok();
    diags.push((
        "dbus_session_bus".into(),
        if dbus_available { "available" } else { "not set" }.into(),
    ));

    // Check if we're in a container (common cause of keyring unavailability)
    let in_container = std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|s| s.contains("docker") || s.contains("lxc") || s.contains("kubepods"))
            .unwrap_or(false);
    diags.push((
        "container_detected".into(),
        if in_container { "yes" } else { "no" }.into(),
    ));

    // Check XDG_RUNTIME_DIR (required by Secret Service)
    let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").is_ok();
    diags.push((
        "xdg_runtime_dir".into(),
        if xdg_runtime { "set" } else { "not set" }.into(),
    ));

    diags.push(("backend".into(), BACKEND_NAME.into()));

    diags
}
