//! Peer-reliability persistence for the desktop presence reader.

use crate::state::{AppState, SharedState};
use crate::state_helpers;

/// Load every community's saved reliability counters from SQLite into
/// the in-memory `peer_reliability` map. Called once on login so the
/// fan-out ranker boots with prior session knowledge instead of
/// treating every peer as neutral.
pub async fn hydrate_peer_reliability(state: &SharedState, pool: &rekindle_db::Db) {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return;
    }
    let owner = owner_key;
    let rows: Result<Vec<(String, String, u32, u32)>, String> =
        crate::db_helpers::db_call(pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT community_id, peer_pseudonym, success_count, failure_count
                 FROM peer_reliability WHERE owner_key = ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![owner], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "peer reliability unreadable; ranking starts neutral");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let mut communities = state.communities.write();
    for (community_id, peer, succ, fail) in rows {
        if let Some(community) = communities.get_mut(&community_id) {
            community.peer_reliability.insert(peer, (succ, fail));
        }
    }
}

/// Drain the dirty set and upsert all pending counters in a single DB
/// transaction. Architecture §14.5: in-memory `peer_reliability` is the
/// source of truth during a session; this batch flush just mirrors it
/// to SQLite so the score survives restarts.
pub async fn flush_peer_reliability(state: &AppState, pool: &rekindle_db::Db) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    if owner_key.is_empty() {
        return;
    }
    let dirty: Vec<(String, String)> = {
        let mut set = state.relay_reliability_dirty.lock();
        if set.is_empty() {
            return;
        }
        set.drain().collect()
    };
    let snapshot: Vec<(String, String, u32, u32)> = {
        let communities = state.communities.read();
        dirty
            .into_iter()
            .filter_map(|(cid, pk)| {
                communities
                    .get(&cid)
                    .and_then(|c| c.peer_reliability.get(&pk))
                    .map(|&(s, f)| (cid, pk, s, f))
            })
            .collect()
    };
    if snapshot.is_empty() {
        return;
    }
    let owner = owner_key;
    let _ = crate::db_helpers::db_call(pool, move |conn| {
        let tx = conn.transaction()?;
        for (cid, pk, s, f) in &snapshot {
            tx.execute(
                "INSERT INTO peer_reliability
                    (owner_key, community_id, peer_pseudonym, success_count, failure_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(owner_key, community_id, peer_pseudonym) DO UPDATE SET
                   success_count = excluded.success_count,
                   failure_count = excluded.failure_count",
                rusqlite::params![owner, cid, pk, i64::from(*s), i64::from(*f)],
            )?;
        }
        tx.commit()
    })
    .await;
}

/// Spawn the periodic flush loop. Idempotent — safe to call multiple
/// times; the loop self-terminates once the user logs out (empty
/// owner key).
pub fn start_peer_reliability_flush(state: SharedState, pool: rekindle_db::Db) {
    crate::state_helpers::spawn_in_login_with_token(
        &state.clone(),
        "peer reliability flush",
        |stop| async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // skip immediate fire
            loop {
                if stop.run_until_cancelled(interval.tick()).await.is_none() {
                    break;
                }
                if state_helpers::owner_key_or_default(&state).is_empty() {
                    break;
                }
                flush_peer_reliability(&state, &pool).await;
            }
        },
    );
}
