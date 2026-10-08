//! The session's `RecordPool` (plan C7.3, D4): created when services start
//! at login, ended at logout after the last record write.
//!
//! The pool runs its Veilid calls on its own scope, not the login scope:
//! logout stops the login scope's tasks first, and those calls are never
//! aborted. At the end the node's `RecordCloser` (the transport node's)
//! closes the records once they finished, off the logout's path
//! (plan C7.6i, C7.6j).

use std::sync::Arc;

use rekindle_db::repo::dht_outbox::OutboxWrite;
use rekindle_lifecycle::SessionScope;
use rekindle_protocol::dht::pool::{RecordPool, UnsentWrite};

use crate::state::AppState;
use crate::state_helpers;

/// Create the session's pool over the node's routing context, ending
/// through the transport node's closer.
///
/// # Errors
/// The node is not running or not adopted, or the safety selection was
/// refused.
pub fn start(state: &Arc<AppState>) -> Result<Arc<RecordPool>, String> {
    let rc = state
        .node
        .read()
        .as_ref()
        .map(|nh| nh.routing_context.clone())
        .ok_or("Veilid node not running")?;
    let transport = state
        .transport
        .read()
        .clone()
        .ok_or("transport node not adopted")?;
    let scope = SessionScope::new(
        "record pool",
        Arc::new(|task| tracing::error!(task, "record pool task panicked")),
    );
    let profile = rekindle_types::config::SafetyProfile::default_dht();
    let pool = RecordPool::new(
        &rc,
        &profile,
        scope,
        state.network_ready.subscribe(),
        transport.record_closer(),
    )
    .map_err(|e| e.to_string())?;
    *state.record_pool.write() = Some(Arc::clone(&pool));
    transport.adopt_records(Some(Arc::clone(&pool)));
    tracing::info!("record pool started");
    Ok(pool)
}

/// Hold the writes this identity's previous logout left unsent (plan
/// C7.6h). Runs right after [`start`], before the session writes anything,
/// so its own newer durable writes replace them slot by slot. The rows are
/// taken (read and deleted), so each is re-pushed once.
pub async fn restore_unsent(state: &Arc<AppState>, pool: &RecordPool) {
    let (Ok(db), Ok(owner)) = (state.db.current(), state_helpers::current_owner_key(state)) else {
        return;
    };
    let taken = crate::db_helpers::db_call(&db, move |conn| {
        rekindle_db::repo::dht_outbox::take(conn, &owner)
    })
    .await;
    match taken {
        Ok(rows) if rows.is_empty() => {}
        Ok(rows) => {
            tracing::info!(
                count = rows.len(),
                "restoring DHT writes the last logout left unsent"
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

/// Save the drained pool's held writes for this identity's next login, and
/// return how many there were.
async fn persist_unsent(state: &Arc<AppState>, unsent: Vec<UnsentWrite>) -> usize {
    let count = unsent.len();
    if count == 0 {
        return 0;
    }
    let (Ok(db), Ok(owner)) = (state.db.current(), state_helpers::current_owner_key(state)) else {
        tracing::warn!(
            count,
            "unsent DHT writes lost: no identity database at logout"
        );
        return 0;
    };
    let rows: Vec<OutboxWrite> = unsent
        .into_iter()
        .map(|write| OutboxWrite {
            record_key: write.record_key,
            subkey: write.subkey,
            data: write.data,
        })
        .collect();
    match crate::db_helpers::db_call(&db, move |conn| {
        rekindle_db::repo::dht_outbox::save(conn, &owner, &rows)
    })
    .await
    {
        Ok(()) => count,
        Err(error) => {
            tracing::warn!(count, %error, "saving the unsent DHT writes failed");
            0
        }
    }
}

/// Begin logout (plan C7.6g): release every task waiting on the pool and
/// refuse new calls, so the login scope stops at once. Calls in flight run
/// on; held writes stay held.
pub fn drain(state: &AppState) {
    if let Some(pool) = state.record_pool.read().clone() {
        pool.drain();
    }
}

/// Admit the teardown's own writes (the Offline status) once the login
/// scope stopped.
pub fn admit_teardown(state: &AppState) {
    if let Some(pool) = state.record_pool.read().clone() {
        pool.admit_teardown();
    }
}

/// Save the held writes and end the session's records: the node's closer
/// closes them once the pool's calls finished, off the logout's path.
/// Returns how many writes were saved for the next login.
pub async fn end(state: &Arc<AppState>) -> usize {
    let Some(pool) = state.record_pool.write().take() else {
        return 0;
    };
    if let Some(transport) = state.transport.read().clone() {
        transport.adopt_records(None);
    }
    // Drain again after the teardown's own writes, so nothing is held after
    // the take below; a write whose wait a drain released was held, not lost.
    pool.drain();
    let unsent = persist_unsent(state, pool.take_held()).await;
    let in_flight = pool.scope().len();
    let calls = pool.scope().running();
    let records = pool.end();
    state.dm_leases.lock().clear();
    state.friend_leases.lock().clear();
    *state.personal_sync_lease.lock() = None;
    tracing::info!(in_flight, ?calls, records, unsent, "record pool ended");
    unsent
}
