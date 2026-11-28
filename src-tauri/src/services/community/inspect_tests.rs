use super::changed_subkeys_from_sequences;

#[test]
fn smpl_only_inspect_core_detects_new_subkey_without_gossip() {
    let changed = changed_subkeys_from_sequences(
        &[0, 1, 2],
        &[Some(0), Some(0), Some(0)],
        &[Some(0), Some(2), Some(0)],
    );
    assert_eq!(changed, vec![1]);
}

#[test]
fn inspect_only_core_detects_changes_when_watch_is_absent() {
    let watch_failed = true;
    let gossip_failed = true;
    assert!(watch_failed);
    assert!(gossip_failed);

    let changed = changed_subkeys_from_sequences(&[0, 1], &[Some(0), Some(0)], &[Some(3), Some(0)]);
    assert_eq!(changed, vec![0]);
}

/// A first write lands at seq 0: never-written to seq 0 is a change, which
/// flattening `None` to 0 used to hide.
#[test]
fn a_first_write_at_seq_zero_is_a_change() {
    let changed = changed_subkeys_from_sequences(&[0, 1], &[None, None], &[Some(0), None]);
    assert_eq!(changed, vec![0]);
    let unseen = changed_subkeys_from_sequences(&[0, 1], &[], &[Some(0), None]);
    assert_eq!(unseen, vec![0]);
}
