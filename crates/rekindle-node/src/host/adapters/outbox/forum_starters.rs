//! Settle forum starters together with their thread-creation dependency.

use rekindle_db::repo::outbox::{self as repo, Row};
use rekindle_db::rusqlite::{self, Transaction};
use rekindle_outbox::{StateChange, StreamKind};
use rekindle_types::message::SendState;
use tokio::sync::broadcast;

pub(super) const CREATION_FAILED: &str = "the thread creation did not land";

pub(super) fn settle(
    tx: &Transaction<'_>,
    owner: &str,
    id: &str,
    state: SendState,
) -> rusqlite::Result<Vec<Row>> {
    let kind = repo::settle_in(tx, owner, id, state)?;
    if kind.as_deref() == Some(StreamKind::GovernanceOp.as_str()) {
        match state {
            SendState::Failed => return repo::abandon_forum_starters(tx, owner, id),
            SendState::Sent => repo::release_forum_starters(tx, owner, id)?,
            _ => {}
        }
    }
    Ok(Vec::new())
}

pub(super) fn on_update(
    tx: &Transaction<'_>,
    owner: &str,
    id: &str,
    state: SendState,
) -> rusqlite::Result<Vec<Row>> {
    if state == SendState::Failed
        && repo::get(tx, owner, id)?
            .is_some_and(|row| row.kind == StreamKind::GovernanceOp.as_str())
    {
        return repo::abandon_forum_starters(tx, owner, id);
    }
    Ok(Vec::new())
}

pub(super) fn publish_abandoned(
    changes: &broadcast::Sender<StateChange>,
    rows: Vec<Row>,
    reason: &str,
) {
    for row in rows {
        if row.state != SendState::Failed {
            let _ = changes.send(StateChange {
                stream: row.stream,
                stream_seq: row.stream_seq,
                client_id: row.client_id,
                kind: StreamKind::ChannelPost,
                state: SendState::Failed,
                wedge_reason: Some(reason.to_owned()),
                retry: None,
            });
        }
    }
}
