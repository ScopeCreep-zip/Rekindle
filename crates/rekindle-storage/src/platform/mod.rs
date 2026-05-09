//! Platform keyring abstraction.
//!
//! Detects OS keyring availability at daemon init time. Used by
//! [`KeyringUnlock`](crate::unlock::keyring::KeyringUnlock) and
//! [`AutoKeyringUnlock`](crate::unlock::auto_keyring::AutoKeyringUnlock)
//! to gate enrollment and unlock attempts.
//!
//! The `keyring` crate (v3) already abstracts Secret Service / Keychain /
//! Credential Manager. These platform modules provide availability detection,
//! platform-specific diagnostics, and the extension point for future direct
//! D-Bus / Keychain / DPAPI access without the `keyring` crate.

pub mod linux;
pub mod macos;
pub mod windows;

const PROBE_SERVICE: &str = "rekindle";
const PROBE_ACCOUNT: &str = "availability-probe";
const PROBE_VALUE: &str = "probe";

/// Test whether the OS keyring is functional on this platform.
///
/// Performs a write → read → delete cycle with a dummy entry. Returns
/// `true` if all three operations succeed. Called once during daemon
/// startup to determine which unlock methods to offer.
///
/// This is deliberately expensive (three keyring round-trips) because
/// it runs exactly once per daemon lifecycle, not per message.
pub fn keyring_available() -> bool {
    let result = keyring::Entry::new(PROBE_SERVICE, PROBE_ACCOUNT).and_then(|entry| {
        entry.set_password(PROBE_VALUE)?;
        let read = entry.get_password()?;
        entry.delete_credential()?;
        if read == PROBE_VALUE {
            Ok(())
        } else {
            Err(keyring::Error::NoEntry)
        }
    });

    match &result {
        Ok(()) => {
            tracing::debug!("platform keyring available");
            true
        }
        Err(e) => {
            tracing::debug!(error = %e, "platform keyring not available");
            false
        }
    }
}

/// Human-readable description of the keyring backend in use.
pub fn keyring_backend_name() -> &'static str {
    #[cfg(target_os = "linux")]
    { linux::BACKEND_NAME }
    #[cfg(target_os = "macos")]
    { macos::BACKEND_NAME }
    #[cfg(target_os = "windows")]
    { windows::BACKEND_NAME }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { "unsupported platform" }
}

/// Platform-specific diagnostic information for `rekindle status --doctor`.
pub fn keyring_diagnostics() -> Vec<(String, String)> {
    #[cfg(target_os = "linux")]
    { linux::diagnostics() }
    #[cfg(target_os = "macos")]
    { macos::diagnostics() }
    #[cfg(target_os = "windows")]
    { windows::diagnostics() }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { vec![("platform".into(), "unsupported".into())] }
}
