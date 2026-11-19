//! `communities.beat_seq` (plan E3.4 S1): our beat count advances by one
//! per beat and survives a restart, so no beat `seq` is reused.

use crate::repo::fixture::{self, OWNER};

#[test]
fn the_beat_seq_advances_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.db");
    {
        let conn = fixture::conn_at(&path);
        fixture::community(&conn, "c");
        assert_eq!(super::next_beat_seq(&conn, OWNER, "c").unwrap(), 1);
        assert_eq!(super::next_beat_seq(&conn, OWNER, "c").unwrap(), 2);
    }
    // A restart: the file reopened, the count resumes where it stood.
    let conn = fixture::conn_at(&path);
    assert_eq!(super::next_beat_seq(&conn, OWNER, "c").unwrap(), 3);
    // A community we hold no row for has no beat to number.
    assert!(super::next_beat_seq(&conn, OWNER, "absent").is_err());
}
