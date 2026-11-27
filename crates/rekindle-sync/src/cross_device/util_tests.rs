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
