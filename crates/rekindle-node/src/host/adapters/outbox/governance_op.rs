//! The `governance_op` stream (plan E3.2, D22): our signed governance op
//! is a row of the community's stream, and its send appends the op to our
//! member record's op log (`M`/2, `rekindle_records::chain::append_at`
//! with `K_M(p)`). One stream per community keeps the chain in op order.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind,
};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::adapters::governance::HostGovernance;

/// The payload: `community length (u16 BE) ‖ community ‖ record length
/// (u16 BE) ‖ record ‖ encoded op`.
fn encode(community: &str, record: &str, op: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(4 + community.len() + record.len() + op.len());
    for text in [community, record] {
        out.extend_from_slice(&u16::try_from(text.len()).ok()?.to_be_bytes());
        out.extend_from_slice(text.as_bytes());
    }
    out.extend_from_slice(op);
    Some(out)
}

fn take_text(payload: &[u8]) -> Option<(&str, &[u8])> {
    let len = usize::from(u16::from_be_bytes(payload.get(..2)?.try_into().ok()?));
    let text = std::str::from_utf8(payload.get(2..2 + len)?).ok()?;
    Some((text, payload.get(2 + len..)?))
}

fn decode(payload: &[u8]) -> Option<(&str, &str, &[u8])> {
    let (community, rest) = take_text(payload)?;
    let (record, op) = take_text(rest)?;
    Some((community, record, op))
}

/// The unlocked identity's `governance_op` streams.
pub struct GovernanceOps(Arc<Outbox<HostOutbox>>);

impl GovernanceOps {
    /// Queue `op` (encoded) for `record`'s op log in `community`.
    ///
    /// # Errors
    /// The op does not decode, a key is too long, or the storage failed.
    pub async fn queue(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
    ) -> Result<(), OutboxError> {
        let decoded = rekindle_codec::governance_op::decode_op(&op)
            .map_err(|e| OutboxError::Storage(format!("governance op: {e}")))?;
        let hash = rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig);
        let payload = encode(community, record, &op)
            .ok_or_else(|| OutboxError::Storage("governance op keys too long".into()))?;
        let entry = Entry {
            kind: StreamKind::GovernanceOp,
            scope: community.to_owned(),
            client_id: hex::encode(hash.0),
            payload,
            deadline: Deadline::Horizon,
        };
        self.0.enqueue(entry, None).await.map(|_| ())
    }
}

/// The stream's sender: the op into our op log.
struct GovernanceOpSender(HostGovernance);

#[async_trait]
impl StreamSender for GovernanceOpSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let Some((community, record, op)) = decode(&row.payload) else {
            return AttemptOutcome::Wedged("unreadable governance op row".into());
        };
        match self.0.append_own_op(community, record, op.to_vec()).await {
            Ok(()) => AttemptOutcome::Sent,
            Err(error) => {
                tracing::debug!(community, %error, "governance op not appended yet");
                AttemptOutcome::Deferred
            }
        }
    }
}

impl OutboxSlot {
    /// Install the `governance_op` sender over `governance`.
    pub(super) fn install_governance_op_sender(&self, governance: HostGovernance) {
        self.senders.install(
            StreamKind::GovernanceOp,
            Arc::new(GovernanceOpSender(governance)),
        );
    }

    /// The running outbox's `governance_op` streams.
    pub(crate) fn governance_ops(&self) -> Option<GovernanceOps> {
        self.current.read().clone().map(GovernanceOps)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_row_payload_round_trips() {
        let payload = super::encode("VLD0:a", "VLD0:m", &[1, 2, 3]).unwrap();
        assert_eq!(
            super::decode(&payload),
            Some(("VLD0:a", "VLD0:m", &[1u8, 2, 3][..]))
        );
    }
}
