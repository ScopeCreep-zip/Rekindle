//! Pure helpers shared by the cross-device sync paths.
//!
//! Centralises `generate_device_id` (pre-port duplicated across
//! `pairing.rs` + `record.rs`) and the `classify_remote_subkey`
//! decode dispatch the watch loop uses to map a raw subkey
//! plaintext into a typed payload.

use rand::RngCore;
use rekindle_types::cross_device_sync::{
    DeviceList, ReadState, SyncManifest, SyncPreferences, SUBKEY_DEVICE_LIST, SUBKEY_MANIFEST,
    SUBKEY_PREFERENCES, SUBKEY_READ_STATE,
};

/// Generate a 16-byte random device ID (hex-encoded). Architecture
/// §28.4 — device IDs are opaque per-identity identifiers; the only
/// requirement is uniqueness across the user's paired devices.
#[must_use]
pub fn generate_device_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Typed decode of a remote personal-sync-record subkey payload.
/// `apply_remote_subkey` in the watch loop dispatches on this; the
/// host applies the per-variant side effect (DB upsert for
/// ReadState, event emit for Preferences / Manifest / DeviceList).
#[derive(Debug, Clone)]
pub enum RemoteSubkeyDecoded {
    ReadState(ReadState),
    Preferences(SyncPreferences),
    Manifest(SyncManifest),
    DeviceList(DeviceList),
}

/// Decode one remote subkey payload. Returns `None` for unknown
/// subkeys or JSON-decode failures (the watch loop silently skips
/// them — pre-port behaviour preserved).
#[must_use]
pub fn classify_remote_subkey(subkey: u32, plaintext: &[u8]) -> Option<RemoteSubkeyDecoded> {
    match subkey {
        SUBKEY_READ_STATE => serde_json::from_slice::<ReadState>(plaintext)
            .ok()
            .map(RemoteSubkeyDecoded::ReadState),
        SUBKEY_PREFERENCES => serde_json::from_slice::<SyncPreferences>(plaintext)
            .ok()
            .map(RemoteSubkeyDecoded::Preferences),
        SUBKEY_MANIFEST => serde_json::from_slice::<SyncManifest>(plaintext)
            .ok()
            .map(RemoteSubkeyDecoded::Manifest),
        SUBKEY_DEVICE_LIST => serde_json::from_slice::<DeviceList>(plaintext)
            .ok()
            .map(RemoteSubkeyDecoded::DeviceList),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_device_id_returns_32_hex_chars() {
        let id = generate_device_id();
        assert_eq!(id.len(), 32, "16 bytes → 32 hex chars");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_device_id_is_unique() {
        // Probabilistic — collision odds for 128-bit IDs are negligible.
        let a = generate_device_id();
        let b = generate_device_id();
        assert_ne!(a, b);
    }

    #[test]
    fn classify_unknown_subkey_returns_none() {
        assert!(classify_remote_subkey(999, b"{}").is_none());
    }

    #[test]
    fn classify_malformed_json_returns_none() {
        assert!(classify_remote_subkey(SUBKEY_READ_STATE, b"\xff\xff bad").is_none());
    }

    #[test]
    fn classify_read_state_round_trips() {
        let rs = ReadState::default();
        let bytes = serde_json::to_vec(&rs).unwrap();
        let Some(RemoteSubkeyDecoded::ReadState(decoded)) =
            classify_remote_subkey(SUBKEY_READ_STATE, &bytes)
        else {
            panic!("expected ReadState variant");
        };
        assert!(decoded.entries.is_empty());
        assert!(decoded.onboarding_complete.is_empty());
    }

    #[test]
    fn classify_preferences_round_trips() {
        let prefs = SyncPreferences {
            theme: Some("dark".to_string()),
            lamport: 42,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&prefs).unwrap();
        let Some(RemoteSubkeyDecoded::Preferences(decoded)) =
            classify_remote_subkey(SUBKEY_PREFERENCES, &bytes)
        else {
            panic!("expected Preferences variant");
        };
        assert_eq!(decoded.lamport, 42);
        assert_eq!(decoded.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn classify_manifest_round_trips() {
        let m = SyncManifest::default();
        let bytes = serde_json::to_vec(&m).unwrap();
        assert!(matches!(
            classify_remote_subkey(SUBKEY_MANIFEST, &bytes),
            Some(RemoteSubkeyDecoded::Manifest(_))
        ));
    }

    #[test]
    fn classify_device_list_round_trips() {
        let dl = DeviceList::default();
        let bytes = serde_json::to_vec(&dl).unwrap();
        assert!(matches!(
            classify_remote_subkey(SUBKEY_DEVICE_LIST, &bytes),
            Some(RemoteSubkeyDecoded::DeviceList(_))
        ));
    }
}
