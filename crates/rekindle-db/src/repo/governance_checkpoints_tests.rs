use super::*;
use crate::repo::fixture::{self, OWNER};

fn row(digest: u8, state: CheckpointState) -> CheckpointRow {
    CheckpointRow {
        height: 1,
        round: u32::from(digest),
        digest: [digest; 32],
        op_hash: [9; 32],
        dir_size: 3,
        state,
    }
}

#[test]
fn a_row_moves_its_state_and_lists_in_order() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    record(&conn, OWNER, "c1", &[row(2, CheckpointState::Proposed)]).unwrap();
    record(
        &conn,
        OWNER,
        "c1",
        &[
            row(1, CheckpointState::Proposed),
            row(2, CheckpointState::Usable),
        ],
    )
    .unwrap();
    assert_eq!(
        list(&conn, OWNER, "c1").unwrap(),
        vec![
            row(1, CheckpointState::Proposed),
            row(2, CheckpointState::Usable)
        ]
    );
}

/// Plan E3.2 S7c.6: a snapshot is kept only on a usable row and read
/// only while the row stays usable; once its checkpoint is equivocated
/// it is not read, though a later rebuild could make it usable again.
#[test]
fn a_snapshot_is_kept_and_read_only_while_its_row_is_usable() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    record(&conn, OWNER, "c1", &[row(1, CheckpointState::Proposed)]).unwrap();
    assert!(!set_snapshot(&conn, OWNER, "c1", 1, &[1; 32], b"s").unwrap());
    record(&conn, OWNER, "c1", &[row(1, CheckpointState::Usable)]).unwrap();
    assert!(set_snapshot(&conn, OWNER, "c1", 1, &[1; 32], b"s").unwrap());
    let read = || usable_snapshot(&conn, OWNER, "c1", 1, &[1; 32]).unwrap();
    assert_eq!(read().as_deref(), Some(b"s".as_slice()));
    record(&conn, OWNER, "c1", &[row(1, CheckpointState::Equivocated)]).unwrap();
    assert_eq!(
        read(),
        None,
        "an equivocated checkpoint's snapshot is not read"
    );
    assert_eq!(
        usable_snapshot(&conn, OWNER, "c1", 1, &[2; 32]).unwrap(),
        None
    );
}

/// Plan E3.2 S7c.6: the FK to `communities`.
#[test]
fn governance_checkpoints_refuses_an_unknown_community() {
    let conn = fixture::conn();
    let err = record(&conn, OWNER, "gone", &[row(1, CheckpointState::Usable)])
        .unwrap_err()
        .to_string();
    assert!(err.contains("FOREIGN KEY constraint failed"), "{err}");
}

/// Plan E3.2 S7c.6: the state CHECK.
#[test]
fn governance_checkpoints_refuses_an_unknown_state() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    let err = conn
        .execute(
            "INSERT INTO governance_checkpoints (owner_key, community_id, height, round, \
                 digest, op_hash, dir_size, state) \
                 VALUES (?1, 'c1', 1, 0, zeroblob(32), zeroblob(32), 0, 'adopted')",
            params![OWNER],
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("CHECK constraint failed"), "{err}");
}
