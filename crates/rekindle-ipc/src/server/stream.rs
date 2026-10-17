//! One subscribed connection's event stream (plan D3.2).
//!
//! Each subscribing connection gets its own ordered stream from the host's
//! journal, taken under the journal's lock: what followed the client's
//! cursor, then `Resumed`, then live frames, with no gap and no duplicate.
//! An event goes out only if the connection still holds `Authenticated`
//! clearance and its filters admit it. The connection's outbound queue is
//! awaited, so a slow client holds only its own stream back; if it falls
//! more than the journal's live capacity behind it is sent `InvalidSession`
//! and its stream ends, so it reloads rather than missing events.

use std::sync::Arc;

use rekindle_types::subscription_events::{Cursor, StreamFrame};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::ServerState;
use crate::framing::encode_frame;
use crate::message::{Message, SecurityLevel, Timestamp, WIRE_VERSION};
use crate::protocol::BusPayload;

/// Start `conn_id`'s stream from `from`, replacing any stream it had.
/// Nothing starts before the host has handed the server its journal (the
/// `Subscribe` was refused then).
pub(super) fn start(
    state: &Arc<ServerState>,
    conn_id: u64,
    tx: mpsc::Sender<Vec<u8>>,
    from: Option<Cursor>,
) {
    let Some(journal) = state.journal.get() else {
        return;
    };
    let stream = journal.subscribe(from);
    let task = state
        .tasks
        .spawn(run(Arc::clone(state), conn_id, tx, stream));
    if let Some(previous) = state.streams.lock().insert(conn_id, task) {
        previous.abort();
    }
}

/// End `conn_id`'s stream (its connection closed).
pub(super) fn stop(state: &ServerState, conn_id: u64) {
    if let Some(task) = state.streams.lock().remove(&conn_id) {
        task.abort();
    }
}

async fn run(
    state: Arc<ServerState>,
    conn_id: u64,
    tx: mpsc::Sender<Vec<u8>>,
    mut stream: rekindle_events::EventStream,
) {
    loop {
        let frame = tokio::select! {
            () = state.stopping.cancelled() => return,
            frame = stream.next() => frame,
        };
        let Some(frame) = frame else {
            return;
        };
        if let StreamFrame::Event(sequenced) = &frame {
            // A lock lowers every client, and the next unlock admits only
            // the clients that identity trusts (plan D1).
            if !state.is_cleared(conn_id).await
                || !state.event_router.read().matches(conn_id, &sequenced.event)
            {
                continue;
            }
        }
        let Some(bytes) = encode(&state, &frame) else {
            continue;
        };
        if tx.send(bytes).await.is_err() {
            return;
        }
    }
}

/// `frame` as the client decodes it.
fn encode(state: &ServerState, frame: &StreamFrame) -> Option<Vec<u8>> {
    let community_scope = match frame {
        StreamFrame::Event(sequenced) => sequenced.event.community().map(String::from),
        StreamFrame::Resumed(_) | StreamFrame::InvalidSession => None,
    };
    let msg: Message<BusPayload> = Message {
        wire_version: WIRE_VERSION,
        msg_id: Uuid::now_v7(),
        correlation_id: None,
        sender: Uuid::nil(),
        timestamp: Timestamp::now(state.epoch),
        payload: BusPayload::Event(frame.clone()),
        security_level: SecurityLevel::Open,
        verified_sender_name: Some("server".to_string()),
        verified_sender_key: None,
        verified_sender_conn: None,
        agent_type: None,
        community_scope,
    };
    match encode_frame(&msg) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            tracing::warn!(error = %e, "event frame encode failed");
            None
        }
    }
}
