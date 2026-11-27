//! Our governance ops' publication states (review RV7 M2): every state
//! change of a `governance_op` op row becomes
//! `GovernanceEvent::OpPublication` from the backend, so the GUI, TUI and
//! CLI show one truth. A stored row is `Pending` (and is announced again at
//! every unlock while it stays queued); a landed row is `Landed` only when
//! the converged op set applies it, else `Failed` with the community's
//! reason; an expired, wedged or discarded row is `Failed`. A refusal at a
//! later convergence is announced by the fates (`publication.rs`).

use std::sync::Arc;

use rekindle_outbox::{Outbox, OutboxDeps as _, OutboxRow, StateChange, StreamKind};
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::OpPublicationState;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::governance_op::is_op_row;
use super::HostOutbox;
use crate::host::adapters::governance::HostGovernance;

/// The community of a `governance_op` stream.
fn community_of(stream: &str) -> Option<&str> {
    stream.split_once(':').map(|(_, scope)| scope)
}

/// What a state change of an op row announces; `None` for `Sent`, whose
/// answer depends on the converged op set.
fn announced(change: &StateChange) -> Option<OpPublicationState> {
    match change.state {
        SendState::Queued => Some(OpPublicationState::Pending),
        SendState::Failed => Some(OpPublicationState::Failed(
            change
                .wedge_reason
                .clone()
                .unwrap_or_else(|| "its row was discarded".to_owned()),
        )),
        _ => None,
    }
}

/// Where a stored op row stands for a window that reads it at load:
/// `Pending` while it waits, `Failed` with its reason; `None` once sent
/// (its landing and any later refusal are the verdict's, announced live).
pub(super) fn row_state(row: &OutboxRow) -> Option<OpPublicationState> {
    match row.state {
        SendState::Queued | SendState::AwaitingKey => Some(OpPublicationState::Pending),
        SendState::Failed => Some(OpPublicationState::Failed(
            row.wedge_reason
                .clone()
                .unwrap_or_else(|| "its row was discarded".to_owned()),
        )),
        _ => None,
    }
}

/// Announce the queued op rows as pending (the unlock, a restart), then
/// every op row's change until `stop`.
pub(super) async fn publish_governance_states(
    governance: HostGovernance,
    outbox: Arc<Outbox<HostOutbox>>,
    mut changes: broadcast::Receiver<StateChange>,
    stop: CancellationToken,
) {
    match outbox
        .deps()
        .prefix_rows(&StreamKind::GovernanceOp.stream(""))
        .await
    {
        Ok(rows) => {
            for row in rows.iter().filter(|r| is_op_row(&r.client_id)) {
                if let Some(community) = community_of(&row.stream) {
                    governance.announce_op(community, &row.client_id, OpPublicationState::Pending);
                    // An op still queued from an earlier session has not
                    // landed: our next write waits for it (plan E3.2
                    // S7d-keys.8, seq n + 1 after seq n).
                    if matches!(row.state, SendState::Queued | SendState::AwaitingKey) {
                        (governance.ctx.communities.unlock_barriers())
                            .of(community)
                            .signed(&row.client_id);
                    }
                }
            }
        }
        Err(error) => tracing::warn!(%error, "queued governance ops not announced"),
    }
    loop {
        // Biased, stop first: a lock beside a queued change announces
        // nothing more (the rows hold every state for the next unlock).
        let change = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            change = changes.recv() => change,
        };
        let change = match change {
            Ok(change) => change,
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "governance op states missed; the rows hold them");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if change.kind != StreamKind::GovernanceOp || !is_op_row(&change.client_id) {
            continue;
        }
        let Some(community) = community_of(&change.stream) else {
            continue;
        };
        // Landed or failed for good, it no longer holds our next write back
        // (plan E3.2 S7d-keys.8: seq n + 1 signs after seq n landed).
        if matches!(change.state, SendState::Sent | SendState::Failed) {
            (governance.ctx.communities.unlock_barriers())
                .of(community)
                .settled(&change.client_id);
        }
        match announced(&change) {
            Some(state) => governance.announce_op(community, &change.client_id, state),
            None if change.state == SendState::Sent => {
                governance
                    .announce_landed(community, &change.client_id)
                    .await;
            }
            None => {}
        }
    }
}

#[cfg(test)]
#[path = "governance_publication_tests.rs"]
mod tests;
