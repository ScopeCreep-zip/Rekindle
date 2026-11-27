//! Storage fault fixtures for the Host's atomic forum post scenarios.

use crate::{CallError, Db};

/// Writes made by forum creation, including its stream allocation.
#[derive(Debug, PartialEq, Eq)]
pub struct Counts {
    pub ops: i64,
    pub head_seq: i64,
    pub threads: i64,
    pub pending_posts: i64,
    pub post_streams: i64,
}

/// Read the affected tables before and after an attempted forum creation.
///
/// # Errors
/// A table could not be read.
pub async fn counts(db: &Db, anchor: &str) -> Result<Counts, CallError> {
    let anchor = anchor.to_owned();
    db.call(move |conn| {
        conn.query_row(
            "SELECT (SELECT count(*) FROM governance_ops WHERE community_id = ?1),
                    (SELECT sum(head_seq) FROM governance_heads WHERE community_id = ?1),
                    (SELECT count(*) FROM community_threads WHERE community_id = ?1),
                    (SELECT count(*) FROM outbox WHERE kind = 'channel_post'),
                    (SELECT count(*) FROM outbox_streams WHERE stream LIKE 'channel_post:%')",
            [&anchor],
            |row| {
                Ok(Counts {
                    ops: row.get(0)?,
                    head_seq: row.get(1)?,
                    threads: row.get(2)?,
                    pending_posts: row.get(3)?,
                    post_streams: row.get(4)?,
                })
            },
        )
    })
    .await
}

/// Reject the channel post insert, the second outbox row of a forum post.
///
/// # Errors
/// The trigger could not be installed.
pub async fn reject_starter(db: &Db) -> Result<(), CallError> {
    db.call(|conn| {
        conn.execute_batch(
            "CREATE TEMP TRIGGER reject_starter BEFORE INSERT ON outbox
             WHEN NEW.kind = 'channel_post'
             BEGIN SELECT RAISE(ABORT, 'starter insert refused'); END;",
        )
    })
    .await
}

/// Restore the durable queue and fail the local echo projection instead.
///
/// # Errors
/// The triggers could not be changed.
pub async fn reject_echo_instead(db: &Db) -> Result<(), CallError> {
    db.call(|conn| {
        conn.execute_batch(
            "DROP TRIGGER reject_starter;
             CREATE TEMP TRIGGER reject_echo BEFORE INSERT ON thread_messages
             BEGIN SELECT RAISE(ABORT, 'local echo store failed'); END;",
        )
    })
    .await
}

/// Move the thread op's deadline into the past. Returns how many attempts
/// the starter has made while its op is still unlanded.
///
/// # Errors
/// The starter or its dependency is missing, or the update failed.
pub async fn expire_creation(db: &Db, message: &str) -> Result<u32, CallError> {
    let id = message.to_owned();
    db.call(move |conn| {
        let (op, attempts): (String, u32) = conn.query_row(
            "SELECT depends_on, attempts FROM outbox WHERE client_id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let changed = conn.execute("UPDATE outbox SET deadline = 0 WHERE client_id = ?1", [&op])?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(attempts)
    })
    .await
}
