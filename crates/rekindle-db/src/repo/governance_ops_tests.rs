use super::*;
use crate::repo::fixture::{self, OWNER};

fn op(byte: u8, lamport: u64, state: OpState) -> StoredOp {
    StoredOp {
        op_hash: [byte; 32],
        author: "aa".into(),
        device_key: "dd".into(),
        device_seq: u64::from(byte),
        lamport,
        issued_at: 7,
        op: vec![byte],
        state,
        signed_here: false,
    }
}

#[test]
fn an_op_is_stored_once_and_lists_by_lamport_and_state() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    assert!(insert(&conn, OWNER, "c1", &op(2, 2, OpState::Applied)).unwrap());
    assert!(insert(&conn, OWNER, "c1", &op(1, 1, OpState::PendingDeps)).unwrap());
    assert!(!insert(&conn, OWNER, "c1", &op(2, 9, OpState::Quarantined)).unwrap());
    let all = list(&conn, OWNER, "c1", None).unwrap();
    assert_eq!(
        all,
        vec![op(1, 1, OpState::PendingDeps), op(2, 2, OpState::Applied)]
    );
    let pending = list(&conn, OWNER, "c1", Some(OpState::PendingDeps)).unwrap();
    assert_eq!(pending, vec![op(1, 1, OpState::PendingDeps)]);
    assert_eq!(load_all(&conn, OWNER).unwrap().len(), 2);
}

#[test]
fn set_state_moves_the_state_and_the_validated_lamport() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    insert(&conn, OWNER, "c1", &op(1, 0, OpState::PendingDeps)).unwrap();
    set_state(&conn, OWNER, "c1", &[1; 32], OpState::Applied, Some(7)).unwrap();
    let got = get_op(&conn, OWNER, "c1", &[1; 32]).unwrap().unwrap();
    assert_eq!(got, op(1, 7, OpState::Applied));
    set_state(&conn, OWNER, "c1", &[1; 32], OpState::Quarantined, None).unwrap();
    let got = get_op(&conn, OWNER, "c1", &[1; 32]).unwrap().unwrap();
    assert_eq!(got, op(1, 7, OpState::Quarantined));
    assert_eq!(get_op(&conn, OWNER, "c1", &[9; 32]).unwrap(), None);
}

#[test]
fn a_head_is_replaced_per_device_key() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    let head = |device: &str, record: &str, seq| DeviceHead {
        device_key: device.into(),
        author: "aa".into(),
        member_record: record.into(),
        head_seq: seq,
        frontier_hash: [3; 32],
    };
    set_head(&conn, OWNER, "c1", &head("d1", "VLD0:m1", 1)).unwrap();
    set_head(&conn, OWNER, "c1", &head("d1", "VLD0:m1", 4)).unwrap();
    // A second device of the same author keeps its own head and record
    // (ADR 0022 rule 3).
    set_head(&conn, OWNER, "c1", &head("d2", "VLD0:m2", 0)).unwrap();
    assert_eq!(
        heads(&conn, OWNER, "c1").unwrap(),
        vec![head("d1", "VLD0:m1", 4), head("d2", "VLD0:m2", 0)]
    );
}

#[test]
fn signed_here_is_stored_and_a_re_insert_never_changes_it() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    let mine = StoredOp {
        signed_here: true,
        ..op(1, 1, OpState::Applied)
    };
    assert!(insert(&conn, OWNER, "c1", &mine).unwrap());
    // The same op back by gossip keeps the local signer's flag.
    assert!(!insert(&conn, OWNER, "c1", &op(1, 1, OpState::Applied)).unwrap());
    assert_eq!(get_op(&conn, OWNER, "c1", &[1; 32]).unwrap(), Some(mine));
    insert(&conn, OWNER, "c1", &op(2, 2, OpState::Applied)).unwrap();
    let loaded: Vec<bool> = load_all(&conn, OWNER)
        .unwrap()
        .into_iter()
        .map(|(_, op)| op.signed_here)
        .collect();
    assert_eq!(loaded, vec![true, false]);
}

#[test]
fn the_signed_here_head_never_counts_ops_re_learnt_from_peers() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    assert_eq!(signed_here_head(&conn, OWNER, "c1", "dd").unwrap(), None);
    let mine = |byte| StoredOp {
        signed_here: true,
        ..op(byte, u64::from(byte), OpState::Applied)
    };
    insert(&conn, OWNER, "c1", &mine(1)).unwrap();
    insert(&conn, OWNER, "c1", &mine(2)).unwrap();
    // Our key's later ops, echoed back by a peer: signed by another copy.
    insert(&conn, OWNER, "c1", &op(5, 5, OpState::Applied)).unwrap();
    insert(&conn, OWNER, "c1", &op(6, 6, OpState::Applied)).unwrap();
    assert_eq!(signed_here_head(&conn, OWNER, "c1", "dd").unwrap(), Some(2));
    assert_eq!(signed_here_head(&conn, OWNER, "c1", "ee").unwrap(), None);
}
