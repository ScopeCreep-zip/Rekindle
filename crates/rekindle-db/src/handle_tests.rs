use super::*;

async fn db() -> Db {
    Db::from(rekindle_asql::Connection::open_in_memory().await.unwrap())
}

#[tokio::test]
async fn current_needs_a_set_database_and_clear_empties_it() {
    let handle = DbHandle::default();
    assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
    handle.set(db().await);
    assert!(handle.current().is_ok());
    handle.clear().await.unwrap();
    assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
    handle.clear().await.unwrap();
}

#[tokio::test]
async fn clear_reports_handles_still_held_and_closes_when_they_drop() {
    let handle = DbHandle::default();
    handle.set(db().await);
    let held = handle.current().unwrap();
    assert_eq!(handle.clear().await, Err(DbStillHeld { others: 1 }));
    assert_eq!(handle.current().unwrap_err(), NotLoggedIn);
    assert_eq!(
        held.call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
#[should_panic(expected = "already set")]
async fn a_second_set_is_a_programming_error() {
    let handle = DbHandle::default();
    handle.set(db().await);
    handle.set(db().await);
}
