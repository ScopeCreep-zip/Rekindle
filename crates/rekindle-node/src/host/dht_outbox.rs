//! The DHT writes a lock left unsent (plan C7.6h, D1): saved at lock and
//! held again at the next unlock of the same identity, by both hosts.
//!
//! Order: at unlock, [`restore_unsent`] runs right after the record pool
//! starts and before the session writes anything, so the session's own
//! newer durable writes replace them slot by slot; the rows are taken
//! (read and deleted), so each is re-pushed once. At lock,
//! [`persist_unsent`] saves what the drained pool still held. E2.5 moves
//! both into `rekindle-outbox` (one outbox owner, D22).

use rekindle_db::repo::dht_outbox::OutboxWrite;
use rekindle_db::Db;
use rekindle_protocol::dht::pool::{RecordPool, UnsentWrite};

/// Hold the writes `owner`'s last lock left unsent.
pub async fn restore_unsent(db: &Db, owner: &str, pool: &RecordPool) {
    let owner = owner.to_string();
    let taken = db
        .call(move |conn| rekindle_db::repo::dht_outbox::take(conn, &owner))
        .await;
    match taken {
        Ok(rows) if rows.is_empty() => {}
        Ok(rows) => {
            tracing::info!(
                count = rows.len(),
                "restoring DHT writes the last lock left unsent"
            );
            pool.restore_held(
                rows.into_iter()
                    .map(|row| UnsentWrite {
                        record_key: row.record_key,
                        subkey: row.subkey,
                        data: row.data,
                    })
                    .collect(),
            );
        }
        Err(error) => tracing::warn!(%error, "reading the unsent DHT writes failed"),
    }
}

/// Save `unsent` for `owner`'s next unlock; how many were saved.
pub async fn persist_unsent(db: &Db, owner: &str, unsent: Vec<UnsentWrite>) -> usize {
    let count = unsent.len();
    if count == 0 {
        return 0;
    }
    let rows: Vec<OutboxWrite> = unsent
        .into_iter()
        .map(|write| OutboxWrite {
            record_key: write.record_key,
            subkey: write.subkey,
            data: write.data,
        })
        .collect();
    let owner = owner.to_string();
    match db
        .call(move |conn| rekindle_db::repo::dht_outbox::save(conn, &owner, &rows))
        .await
    {
        Ok(()) => count,
        Err(error) => {
            tracing::warn!(count, %error, "saving the unsent DHT writes failed");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fixture::Fixture;

    /// What a lock saves is what the next unlock takes, once.
    #[tokio::test]
    async fn saved_writes_are_taken_once() {
        let f = Fixture::new();
        let made = crate::host::identity::create(&f.root, &f.stores(), None, "pass", 1)
            .await
            .unwrap();
        let owner = made.public_key.clone();
        drop(made);
        let db = f.db.current().unwrap();
        let unsent = vec![UnsentWrite {
            record_key: "VLD0:key".into(),
            subkey: 3,
            data: vec![1, 2, 3],
        }];
        assert_eq!(persist_unsent(&db, &owner, unsent).await, 1);
        let o = owner.clone();
        let first = db
            .call(move |conn| rekindle_db::repo::dht_outbox::take(conn, &o))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].subkey, 3);
        let second = db
            .call(move |conn| rekindle_db::repo::dht_outbox::take(conn, &owner))
            .await
            .unwrap();
        assert!(second.is_empty());
    }
}
