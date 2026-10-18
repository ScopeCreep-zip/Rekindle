//! The session's records (plan C7.3, D4, C9.1e): the transport node's
//! session pool, started when services start at login and ended at logout
//! after the last record write.
//!
//! The pool runs its Veilid calls on its own scope, not the login scope:
//! logout stops the login scope's tasks first, and those calls are never
//! aborted. At the end the node's closer closes the records once they
//! finished, off the logout's path (plan C7.6i, C7.6j).

use std::sync::Arc;

use rekindle_records::port::{RecordPort, UnsentWrite};

use crate::state::AppState;
use crate::state_helpers;

/// Start the session's records on the transport node.
///
/// # Errors
/// The transport node is not adopted, or refused a session.
pub fn start(state: &Arc<AppState>) -> Result<Arc<dyn RecordPort>, String> {
    let transport = state
        .transport
        .read()
        .clone()
        .ok_or("transport node not adopted")?;
    let pool = transport.start_records().map_err(|e| e.to_string())?;
    *state.record_pool.write() = Some(Arc::clone(&pool));
    tracing::info!("record pool started");
    Ok(pool)
}

/// Hold the writes this identity's previous logout left unsent (plan
/// C7.6h): `host::dht_outbox::restore_unsent`, right after [`start`]; and
/// re-register the queued posts waiting on them.
pub async fn restore_unsent(state: &Arc<AppState>, pool: &dyn RecordPort) {
    let (Ok(db), Ok(owner)) = (state.db.current(), state_helpers::current_owner_key(state)) else {
        return;
    };
    rekindle_node::host::dht_outbox::restore_unsent(&db, &owner, pool).await;
    // The posts waiting on those writes wait on them again, so a restored
    // write's settle moves them to sent or failed (plan D1, C7.14).
    let held = crate::db_helpers::db_call(&db, move |conn| {
        rekindle_db::repo::messages::held_channel_messages(conn, &owner)
    })
    .await;
    match held {
        Ok(held) => {
            let mut pending = state.channel_pending_deliveries.lock();
            for message in held {
                pending
                    .entry((message.record_key, message.subkey))
                    .or_default()
                    .push(rekindle_channel::deps::PendingDelivery {
                        community: message.community_id,
                        channel: message.channel_id,
                        message: message.message_id,
                    });
            }
        }
        Err(error) => tracing::warn!(%error, "reading the queued posts failed"),
    }
}

/// Save the drained pool's held writes for this identity's next login, and
/// return how many there were.
async fn persist_unsent(state: &Arc<AppState>, unsent: Vec<UnsentWrite>) -> usize {
    if unsent.is_empty() {
        return 0;
    }
    let (Ok(db), Ok(owner)) = (state.db.current(), state_helpers::current_owner_key(state)) else {
        tracing::warn!(
            count = unsent.len(),
            "unsent DHT writes lost: no identity database at logout"
        );
        return 0;
    };
    rekindle_node::host::dht_outbox::persist_unsent(&db, &owner, unsent).await
}

/// Begin logout (plan C7.6g): release every task waiting on the pool and
/// refuse new calls, so the login scope stops at once. Calls in flight run
/// on; held writes stay held.
pub fn drain(state: &AppState) {
    if let Some(transport) = state.transport.read().clone() {
        transport.drain_records();
    }
}

/// Admit the teardown's own writes (the Offline status) once the login
/// scope stopped.
pub fn admit_teardown(state: &AppState) {
    if let Some(transport) = state.transport.read().clone() {
        transport.admit_records_teardown();
    }
}

/// Save the held writes and end the session's records: the node's closer
/// closes them once the pool's calls finished, off the logout's path.
/// Returns how many writes were saved for the next login.
pub async fn end(state: &Arc<AppState>) -> usize {
    if state.record_pool.write().take().is_none() {
        return 0;
    }
    let Some(transport) = state.transport.read().clone() else {
        return 0;
    };
    // Drained again after the teardown's own writes, so nothing is held
    // after the take; a write whose wait a drain released was held, not lost.
    let unsent = persist_unsent(state, transport.take_unsent()).await;
    transport.end_records();
    state.dm_leases.lock().clear();
    state.friend_leases.lock().clear();
    *state.personal_sync_lease.lock() = None;
    tracing::info!(unsent, "record pool ended");
    unsent
}
