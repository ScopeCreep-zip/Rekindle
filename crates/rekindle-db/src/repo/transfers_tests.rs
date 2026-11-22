use super::*;
use crate::repo::fixture::{self, OWNER};

fn row(id: &str, state: TransferState, started_at: i64) -> TransferRow {
    TransferRow {
        attachment_id: id.into(),
        target: TransferTarget::Channel {
            community: "c".into(),
            channel_id: "ch".into(),
        },
        state,
        chunks_done: 0,
        chunk_count: 0,
        bytes_done: 0,
        total: 0,
        dest_name: "a.png".into(),
        dest_path: "/d/a.png".into(),
        started_at,
        finished_at: None,
    }
}

#[test]
fn a_restarted_transfer_keeps_its_checkpoint() {
    let conn = fixture::conn();
    begin(&conn, OWNER, &row("a", TransferState::Active, 5)).unwrap();
    checkpoint(&conn, OWNER, "a", (3, 8), (3_000, 8_000)).unwrap();
    assert!(set_state(&conn, OWNER, "a", TransferState::Paused, 6).unwrap());
    begin(&conn, OWNER, &row("a", TransferState::Active, 9)).unwrap();
    let got = get(&conn, OWNER, "a").unwrap().unwrap();
    assert_eq!(got.state, TransferState::Active);
    assert_eq!((got.chunks_done, got.chunk_count), (3, 8));
    assert_eq!((got.bytes_done, got.total), (3_000, 8_000));
    assert_eq!(got.started_at, 5);
    assert_eq!(got.finished_at, None);
}

#[test]
fn done_is_stamped_and_the_list_is_newest_first() {
    let conn = fixture::conn();
    begin(&conn, OWNER, &row("a", TransferState::Active, 1)).unwrap();
    begin(&conn, OWNER, &row("b", TransferState::Queued, 2)).unwrap();
    assert!(set_state(&conn, OWNER, "a", TransferState::Done, 7).unwrap());
    let all = list(&conn, OWNER).unwrap();
    assert_eq!(
        all.iter()
            .map(|r| r.attachment_id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(all[1].finished_at, Some(7));
    assert!(!set_state(&conn, OWNER, "zz", TransferState::Done, 7).unwrap());
}

#[test]
fn an_interrupted_transfer_is_paused_at_unlock() {
    let conn = fixture::conn();
    begin(&conn, OWNER, &row("a", TransferState::Active, 1)).unwrap();
    begin(&conn, OWNER, &row("b", TransferState::Queued, 2)).unwrap();
    begin(&conn, OWNER, &row("c", TransferState::Done, 3)).unwrap();
    assert_eq!(pause_interrupted(&conn, OWNER).unwrap(), 2);
    assert_eq!(
        get(&conn, OWNER, "a").unwrap().unwrap().state,
        TransferState::Paused
    );
    assert_eq!(
        get(&conn, OWNER, "c").unwrap().unwrap().state,
        TransferState::Done
    );
    assert!(remove(&conn, OWNER, "a").unwrap());
    assert!(get(&conn, OWNER, "a").unwrap().is_none());
}
