//! The `governance_op` stream (plan E3.2, D22): our signed governance op
//! is a row of the community's stream, and its send appends the op to our
//! member record's op log (`M`/2, `rekindle_records::chain::append_at`
//! with `K_M(p)`). One stream per community keeps the chain in op order.
//! [`GovernanceOps::publish`] returns when the op's first attempt landed
//! on `M`/2 and `M`/3 ([`Published::Landed`]), or with the reason it did
//! not ([`Published::Queued`]: the row stays and the outbox retries it, so
//! the op is ours and the action stands): a joiner's walk reads what is on
//! the DHT.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{AttemptOutcome, Deadline, Entry, Outbox, OutboxRow, StreamKind};

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

/// Whether a published op landed, for its publisher.
type Landed = tokio::sync::oneshot::Sender<Result<(), String>>;

/// The publishers waiting on an op's first attempt, by community and the
/// op's client id (its hash), and those waiting until it landed through
/// the retries ([`GovernanceOps::publish_landed`]).
#[derive(Default)]
pub struct OpWaiters {
    first: parking_lot::Mutex<BTreeMap<(String, String), Landed>>,
    landing: parking_lot::Mutex<BTreeMap<(String, String), tokio::sync::oneshot::Sender<()>>>,
}

impl OpWaiters {
    fn wait(
        &self,
        community: &str,
        client_id: &str,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.first
            .lock()
            .insert((community.to_owned(), client_id.to_owned()), tx);
        rx
    }

    fn wait_landed(&self, community: &str, client_id: &str) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.landing
            .lock()
            .insert((community.to_owned(), client_id.to_owned()), tx);
        rx
    }

    fn forget(&self, community: &str, client_id: &str) {
        let key = (community.to_owned(), client_id.to_owned());
        self.first.lock().remove(&key);
        self.landing.lock().remove(&key);
    }

    fn landed(&self, community: &str, client_id: &str) {
        let key = (community.to_owned(), client_id.to_owned());
        let waiter = self.first.lock().remove(&key);
        if let Some(waiter) = waiter {
            drop(waiter.send(Ok(())));
        }
        let waiter = self.landing.lock().remove(&key);
        if let Some(waiter) = waiter {
            // A publisher that stopped waiting (its deadline passed) is gone.
            waiter.send(()).ok();
        }
    }

    /// The stream is held at an op that did not land: no later op of
    /// `community` lands before it, so every publisher there hears why.
    fn failed(&self, community: &str, reason: &str) {
        let waiting: Vec<Landed> = {
            let mut map = self.first.lock();
            let keys: Vec<_> = map
                .keys()
                .filter(|(c, _)| c == community)
                .cloned()
                .collect();
            keys.iter().filter_map(|key| map.remove(key)).collect()
        };
        for waiter in waiting {
            drop(waiter.send(Err(reason.to_owned())));
        }
    }

    /// The outbox stopped: every publisher hears it (the rows stay).
    pub(super) fn clear(&self) {
        self.first.lock().clear();
        self.landing.lock().clear();
    }
}

/// The unlocked identity's `governance_op` streams.
/// What became of a published op's first attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Published {
    /// On `M`/2, with our `Frontier` on `M`/3.
    Landed,
    /// Not on the DHT yet, for the reason given; the outbox retries it.
    Queued(String),
}

pub struct GovernanceOps {
    outbox: Arc<Outbox<HostOutbox>>,
    waiters: Arc<OpWaiters>,
}

impl GovernanceOps {
    /// Queue `op` (encoded) for `record`'s op log in `community`, and wait
    /// for its first attempt: [`Published::Landed`] when it is on `M`/2
    /// and our `Frontier` on `M`/3, [`Published::Queued`] when it is not
    /// there yet (the row stays queued and the outbox retries it).
    ///
    /// # Errors
    /// The op does not decode, a key is too long, the storage failed, or
    /// the outbox stopped before the first attempt.
    pub async fn publish(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
    ) -> Result<Published, String> {
        let (entry, client_id) = row(community, record, &op)?;
        let first = self.waiters.wait(community, &client_id);
        self.enqueue(entry, community, &client_id).await?;
        let first = first
            .await
            .map_err(|_| "the outbox stopped before the op landed".to_owned())?;
        Ok(first.map_or_else(Published::Queued, |()| Published::Landed))
    }

    /// Queue `op` as [`publish`](Self::publish) does, and wait until it
    /// landed on `M`/2 with our `Frontier` on `M`/3, through the outbox's
    /// retries, for at most `within` (plan E3.2 S6 (4)).
    ///
    /// # Errors
    /// As [`publish`](Self::publish), or the op is not yet published after
    /// `within`: its row stays queued and the outbox lands it later.
    pub async fn publish_landed(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
        within: std::time::Duration,
    ) -> Result<(), String> {
        let (entry, client_id) = row(community, record, &op)?;
        let landed = self.waiters.wait_landed(community, &client_id);
        self.enqueue(entry, community, &client_id).await?;
        let waited = tokio::time::timeout(within, landed).await;
        self.waiters.forget(community, &client_id);
        match waited {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("the outbox stopped before the op landed".to_owned()),
            Err(_) => Err("not yet published: the op is queued until it lands".to_owned()),
        }
    }

    async fn enqueue(&self, entry: Entry, community: &str, client_id: &str) -> Result<(), String> {
        match self.outbox.enqueue(entry, None).await {
            Ok(_) => Ok(()),
            Err(error) => {
                self.waiters.forget(community, client_id);
                Err(error.to_string())
            }
        }
    }
}

/// `op`'s outbox entry for `record`'s op log in `community`, and its
/// client id (the op's hash).
fn row(community: &str, record: &str, op: &[u8]) -> Result<(Entry, String), String> {
    let decoded =
        rekindle_codec::governance_op::decode_op(op).map_err(|e| format!("governance op: {e}"))?;
    let hash = rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig);
    let payload = encode(community, record, op).ok_or("governance op keys too long")?;
    let client_id = hex::encode(hash.0);
    let entry = Entry {
        kind: StreamKind::GovernanceOp,
        scope: community.to_owned(),
        client_id: client_id.clone(),
        payload,
        deadline: Deadline::Horizon,
    };
    Ok((entry, client_id))
}

/// The stream's sender: the op into our op log.
struct GovernanceOpSender {
    governance: HostGovernance,
    waiters: Arc<OpWaiters>,
}

#[async_trait]
impl StreamSender for GovernanceOpSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let Some((community, record, op)) = decode(&row.payload) else {
            if let Some(community) = row.stream.split_once(':').map(|(_, scope)| scope) {
                self.waiters
                    .failed(community, "unreadable governance op row");
            }
            return AttemptOutcome::Wedged("unreadable governance op row".into());
        };
        match self
            .governance
            .append_own_op(community, record, op.to_vec())
            .await
        {
            Ok(()) => {
                self.waiters.landed(community, &row.client_id);
                AttemptOutcome::Sent
            }
            Err(error) => {
                tracing::debug!(community, %error, "governance op not appended yet");
                self.waiters.failed(community, &error);
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
            Arc::new(GovernanceOpSender {
                governance,
                waiters: Arc::clone(&self.governance_waiters),
            }),
        );
    }

    /// The running outbox's `governance_op` streams.
    pub(crate) fn governance_ops(&self) -> Option<GovernanceOps> {
        let outbox = self.current.read().clone()?;
        Some(GovernanceOps {
            outbox,
            waiters: Arc::clone(&self.governance_waiters),
        })
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
