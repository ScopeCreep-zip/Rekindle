//! The `bootstrap` stream (plan E3.2 join step 5, D22; ADR 0011 "joining
//! works with nobody online"): our `MemberJoined` handed to the inviter
//! when no member answered at the join. The row of the stream of
//! `<community>` holds the inviter's member record and the op's hash (the
//! op itself is in our op store). Each attempt reads the inviter's
//! current route from its `M`/6 and makes the exchange
//! (`HostGovernance::bootstrap_exchange`); only an answered frontier
//! settles the row `delivered`. A refusal withdraws the join as the join
//! itself does; our leave or removal discards the row.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind,
};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::adapters::governance::HostGovernance;

/// The reason a row of a community we left settles with.
pub const LEFT: &str = "left";

/// The payload: `op hash (32) ‖ inviter member record`.
fn encode(hash: &[u8; 32], inviter_record: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + inviter_record.len());
    out.extend_from_slice(hash);
    out.extend_from_slice(inviter_record.as_bytes());
    out
}

fn decode(payload: &[u8]) -> Option<([u8; 32], &str)> {
    let hash = payload.get(..32)?.try_into().ok()?;
    let record = std::str::from_utf8(payload.get(32..)?).ok()?;
    (!record.is_empty()).then_some((hash, record))
}

/// The inviter's route read for an attempt, when it may be called: none
/// read, or a blob the transport knows dead (plan RV1g B3: Veilid's
/// `RouteChange` or two timed-out calls, until a newer `M`/6 or the failure
/// cache's expiry), defers the row without a call.
fn callable(
    read: Option<Vec<u8>>,
    dead: impl FnOnce(&[u8]) -> bool,
) -> Result<Vec<u8>, AttemptOutcome> {
    match read {
        Some(route) if dead(&route) => {
            tracing::debug!("bootstrap exchange deferred: the inviter's route is known dead");
            Err(AttemptOutcome::Deferred)
        }
        Some(route) => Ok(route),
        None => Err(AttemptOutcome::Deferred),
    }
}

struct BootstrapSender(HostGovernance);

#[async_trait]
impl StreamSender for BootstrapSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let Some(community) = row.stream.strip_prefix("bootstrap:") else {
            return AttemptOutcome::Wedged(format!("not a bootstrap stream: {}", row.stream));
        };
        let Some((hash, inviter_record)) = decode(&row.payload) else {
            return AttemptOutcome::Wedged("malformed bootstrap row".into());
        };
        let governance = &self.0;
        if !governance.holds_community(community) {
            // Our leave or removal discards the row; one reaching here met
            // a community we do not hold (finding JW2: attempted before the
            // unlock restored it), so it is said, not settled silently.
            tracing::warn!(
                community,
                "bootstrap row of a community we do not hold: settled left"
            );
            return AttemptOutcome::Superseded(LEFT.to_owned());
        }
        let Some(joined) = governance.held_joined(community, &hash).await else {
            return AttemptOutcome::Wedged("our MemberJoined is not held".into());
        };
        let read = governance.inviter_route(community, inviter_record).await;
        let route = match callable(read, |blob| governance.route_declared_dead(blob)) {
            Ok(route) => route,
            Err(outcome) => return outcome,
        };
        use crate::host::adapters::governance::Exchange;
        match (governance)
            .bootstrap_exchange(community, &route, inviter_record, &joined)
            .await
        {
            Exchange::Merged => {
                tracing::info!(community, "bootstrap exchange answered: the join is known");
                AttemptOutcome::Receipted
            }
            Exchange::Refused(refused) => {
                governance
                    .withdraw_refused_join(community, &hash, &refused)
                    .await;
                AttemptOutcome::Superseded(refused.to_string())
            }
            Exchange::Unanswered(reason) => {
                tracing::debug!(community, %reason, "bootstrap exchange not answered");
                AttemptOutcome::Deferred
            }
        }
    }
}

/// The unlocked identity's `bootstrap` streams.
pub struct Bootstraps(Arc<Outbox<HostOutbox>>);

impl Bootstraps {
    /// Commit the exchange of our `MemberJoined` (`hash`) of `community`
    /// with the inviter whose record is `inviter_record`.
    ///
    /// # Errors
    /// The storage failed, or the row is already queued.
    pub async fn queue(
        &self,
        community: &str,
        inviter_record: &str,
        hash: &[u8; 32],
    ) -> Result<(), OutboxError> {
        let entry = Entry {
            kind: StreamKind::Bootstrap,
            scope: community.to_owned(),
            client_id: format!("bootstrap:{community}:{}", hex::encode(hash)),
            payload: encode(hash, inviter_record),
            deadline: Deadline::Horizon,
        };
        self.0.enqueue(entry, None).await.map(drop)
    }

    /// We left `community`: discard its row; how many.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard(&self, community: &str) -> Result<usize, OutboxError> {
        (self.0)
            .discard_streams(StreamKind::Bootstrap, community, LEFT)
            .await
    }
}

impl OutboxSlot {
    /// The running outbox's `bootstrap` streams.
    pub(crate) fn bootstraps(&self) -> Option<Bootstraps> {
        self.current.read().clone().map(Bootstraps)
    }

    /// Hand the community streams (`mek_transfer`, `bootstrap`) to their
    /// senders once the unlock restored every community and its stored keys
    /// (unlock step 3e'''). Both read the community from that cache: one
    /// attempted while it is still empty finds no community and would
    /// settle as left or departed, so the exchange the last lock left
    /// unanswered was dropped for good (finding JW2). Until then their rows
    /// defer (`Senders::send` with no sender, kept with their backoff).
    /// Each held community's `bootstrap` row is then made due at once: the
    /// unlock re-drives the exchange rather than waiting out its backoff.
    pub(crate) async fn hand_over_communities(&self, governance: HostGovernance) {
        let communities = governance.held_communities();
        self.install_mek_transfer_sender(governance.clone());
        (self.senders).install(StreamKind::Bootstrap, Arc::new(BootstrapSender(governance)));
        let Some(outbox) = self.current.read().clone() else {
            return;
        };
        for community in communities {
            if let Err(error) = outbox
                .bring_forward(StreamKind::Bootstrap, &community)
                .await
            {
                tracing::warn!(%error, "bootstrap exchange not brought forward at unlock");
            }
        }
    }
}

#[cfg(test)]
mod tests;
