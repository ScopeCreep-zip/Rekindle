use super::*;

#[test]
fn merge_tightens_constraints() {
    let mut active = PolicyConfig {
        min_hop_count: Some(1),
        max_gossip_ttl: Some(5),
    };
    let loaded = PolicyConfig {
        min_hop_count: Some(2),
        max_gossip_ttl: Some(3),
    };
    merge(&mut active, &loaded);
    assert_eq!(active.min_hop_count, Some(2)); // higher = more privacy
    assert_eq!(active.max_gossip_ttl, Some(3)); // lower = more restrictive
}

#[test]
fn merge_does_not_loosen() {
    let mut active = PolicyConfig {
        min_hop_count: Some(3),
        max_gossip_ttl: Some(2),
    };
    let loaded = PolicyConfig {
        min_hop_count: Some(1),
        max_gossip_ttl: Some(8),
    };
    merge(&mut active, &loaded);
    assert_eq!(active.min_hop_count, Some(3)); // not lowered
    assert_eq!(active.max_gossip_ttl, Some(2)); // not raised
}

#[test]
fn absent_layer_contributes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_layer(&dir.path().join("policy.toml")).unwrap(), None);
}

/// A present but malformed policy is an error, not a default.
#[test]
fn malformed_layer_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.toml");
    std::fs::write(&path, "min_hop_count = \"two\"").unwrap();
    assert!(matches!(load_layer(&path), Err(PolicyError::Parse { .. })));
    std::fs::write(&path, "unknown_field = 1").unwrap();
    assert!(matches!(load_layer(&path), Err(PolicyError::Parse { .. })));
}
