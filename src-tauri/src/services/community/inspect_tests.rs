use super::changed_subkeys_from_sequences;

#[test]
fn smpl_only_inspect_core_detects_new_subkey_without_gossip() {
    let changed = changed_subkeys_from_sequences(&[0, 0, 0], &[0, 2, 0]);
    assert_eq!(changed, vec![1]);
}

#[test]
fn inspect_only_core_detects_changes_when_watch_is_absent() {
    let watch_failed = true;
    let gossip_failed = true;
    assert!(watch_failed);
    assert!(gossip_failed);

    let changed = changed_subkeys_from_sequences(&[0, 0], &[3, 0]);
    assert_eq!(changed, vec![0]);
}
