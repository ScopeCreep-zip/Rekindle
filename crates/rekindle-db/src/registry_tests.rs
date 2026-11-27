use super::*;

fn entry(pk: &str) -> IdentityEntry {
    IdentityEntry {
        public_key: pk.into(),
        display_name: format!("name-{pk}"),
        created_at: 1,
    }
}

#[test]
fn a_registry_round_trips_and_a_missing_one_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    assert_eq!(load(&path).unwrap(), Registry::default());
    let mut reg = Registry::default();
    reg.upsert(entry("a"));
    reg.upsert(entry("b"));
    reg.upsert(IdentityEntry {
        display_name: "renamed".into(),
        ..entry("a")
    });
    save(&path, &reg).unwrap();
    let back = load(&path).unwrap();
    assert_eq!(back, reg);
    assert_eq!(back.get("a").unwrap().display_name, "renamed");
    assert_eq!(back.identities.len(), 2);
    let mut back = back;
    assert!(back.remove("a"));
    assert!(!back.remove("a"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn a_malformed_registry_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identities.json");
    std::fs::write(&path, b"not json").unwrap();
    assert!(matches!(load(&path), Err(RegistryError::Malformed(_))));
}
