use super::*;

fn vault() -> (tempfile::TempDir, VaultStore) {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = VaultStore::create(
        &dir.path().join("d.vault"),
        rekindle_test_fixtures::PASSPHRASE,
    )
    .unwrap();
    (dir, store)
}

/// A second device's entry holds its key, `M(p,d)` and `K_M(p,d)` (ADR 0022
/// rule 4); the key reads back unchanged once the record is bound.
#[test]
fn a_bound_record_and_its_owner_read_back_beside_the_key() {
    let (_dir, device) = vault();
    store_device_op_key(&device, "c", &[7; 32]).unwrap();
    assert!(load_device_record(&device, "c").unwrap().is_none());
    store_device_record(&device, "c", "VLD0:mpd", "owner:secret").unwrap();
    let keys = load_device_record(&device, "c").unwrap().unwrap();
    assert_eq!(keys.record, "VLD0:mpd");
    assert_eq!(keys.owner.as_str(), "owner:secret");
    assert_eq!(*load_device_op_key(&device, "c").unwrap().unwrap(), [7; 32]);
}

/// A record belongs to a key: with no key held, nothing is bound.
#[test]
fn a_record_without_a_held_key_is_refused() {
    let (_dir, device) = vault();
    assert!(store_device_record(&device, "c", "VLD0:mpd", "o").is_err());
    assert!(load_device_record(&device, "c").unwrap().is_none());
}

/// A truncated entry is refused, never read as a key alone.
#[test]
fn a_truncated_entry_is_malformed() {
    let mut bytes = vec![7; 32];
    bytes.extend_from_slice(&5u16.to_be_bytes());
    bytes.extend_from_slice(b"VLD0:");
    assert!(parse_entry(&bytes).is_err());
    assert!(parse_entry(&[7; 31]).is_err());
}
