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
            return AttemptOutcome::Superseded(LEFT.to_owned());
        }
        let Some(joined) = governance.held_joined(community, &hash).await else {
            return AttemptOutcome::Wedged("our MemberJoined is not held".into());
        };
        let Some(route) = governance.inviter_route(community, inviter_record).await else {
            return AttemptOutcome::Deferred;
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
                governance.withdraw_refused_join(community, &refused).await;
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

    pub(super) fn install_bootstrap_sender(&self, governance: HostGovernance) {
        (self.senders).install(StreamKind::Bootstrap, Arc::new(BootstrapSender(governance)));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_payload_round_trips_its_hash_and_record() {
        let payload = super::encode(&[7; 32], "VLD0:rec");
        assert_eq!(super::decode(&payload), Some(([7; 32], "VLD0:rec")));
        assert_eq!(super::decode(&payload[..32]), None);
        assert_eq!(super::decode(&payload[..10]), None);
    }
}
