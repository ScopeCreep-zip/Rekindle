//! The `governance_op` stream (plan E3.2, D22): our signed governance op
//! is a row of the community's stream, and its send appends the op to our
//! member record's op log (`M`/2, `rekindle_records::chain::append_at`
//! with `K_M(p)`). One stream per community keeps the chain in op order.
//! [`GovernanceOps::publish_answer`] returns when the op's first attempt landed
//! on `M`/2 and `M`/3 ([`Published::Landed`]), or with the reason it did
//! not ([`Published::Queued`]: the row stays and the outbox retries it, so
//! the op is ours and the action stands): a joiner's walk reads what is on
//! the DHT. An invite's `InviteCreated` goes in one transaction with its
//! revoke-unless-landed row behind it (review RV4 M3), so no crash or
//! storage error between the two leaves a live invite nobody was handed.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{AttemptOutcome, Deadline, Entry, Outbox, OutboxRow, StreamKind};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::adapters::governance::HostGovernance;

/// A row's payload: `tag ‖ community length (u16 BE) ‖ community ‖ …`, the
/// rest by the tag: [`OP_ROW`] `record length (u16 BE) ‖ record ‖ encoded
/// op`, [`REVOKE_ROW`] the invite id (16 bytes).
const OP_ROW: u8 = 0;
const REVOKE_ROW: u8 = 1;

/// The client id of `invite_id`'s revoke-unless-landed row.
fn revoke_client_id(invite_id: [u8; 16]) -> String {
    format!("revoke-unless-landed:{}", hex::encode(invite_id))
}

/// Whether `client_id` is an op row's (an op's hash), not a revoke row's.
pub(super) fn is_op_row(client_id: &str) -> bool {
    !client_id.starts_with("revoke-unless-landed:")
}

fn push_text(out: &mut Vec<u8>, text: &str) -> Option<()> {
    out.extend_from_slice(&u16::try_from(text.len()).ok()?.to_be_bytes());
    out.extend_from_slice(text.as_bytes());
    Some(())
}

fn encode(community: &str, record: &str, op: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(5 + community.len() + record.len() + op.len());
    out.push(OP_ROW);
    push_text(&mut out, community)?;
    push_text(&mut out, record)?;
    out.extend_from_slice(op);
    Some(out)
}

fn encode_revoke(community: &str, invite_id: [u8; 16]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(19 + community.len());
    out.push(REVOKE_ROW);
    push_text(&mut out, community)?;
    out.extend_from_slice(&invite_id);
    Some(out)
}

fn take_text(payload: &[u8]) -> Option<(&str, &[u8])> {
    let len = usize::from(u16::from_be_bytes(payload.get(..2)?.try_into().ok()?));
    let text = std::str::from_utf8(payload.get(2..2 + len)?).ok()?;
    Some((text, payload.get(2 + len..)?))
}

/// A decoded row.
#[derive(Debug, PartialEq, Eq)]
enum RowBody<'a> {
    Op {
        community: &'a str,
        record: &'a str,
        op: &'a [u8],
    },
    RevokeUnlessLanded {
        community: &'a str,
        invite_id: [u8; 16],
    },
}

fn decode(payload: &[u8]) -> Option<RowBody<'_>> {
    let (&tag, rest) = payload.split_first()?;
    let (community, rest) = take_text(rest)?;
    match tag {
        OP_ROW => {
            let (record, op) = take_text(rest)?;
            Some(RowBody::Op {
                community,
                record,
                op,
            })
        }
        REVOKE_ROW => Some(RowBody::RevokeUnlessLanded {
            community,
            invite_id: rest.try_into().ok()?,
        }),
        _ => None,
    }
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
    /// Queue `op` (encoded) for `record`'s op log in `community` and wait
    /// for its first attempt for at most `within` (plan E3.2 S6b, RV4 review
    /// M2). [`Published::Queued`]: the row stays and the outbox lands it; its
    /// state reaches the frontends as `GovernanceEvent::OpPublication`
    /// (`governance_publication`).
    ///
    /// # Errors
    /// The op does not decode, a key is too long, the storage failed, or
    /// the outbox stopped before the first attempt.
    pub async fn publish_answer(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
        within: std::time::Duration,
    ) -> Result<Published, String> {
        let (entry, client_id) = row(community, record, &op)?;
        let first = self.waiters.wait(community, &client_id);
        self.enqueue(entry, community, &client_id).await?;
        let first = match tokio::time::timeout(within, first).await {
            Ok(Ok(first)) => first,
            Ok(Err(_)) => return Err("the outbox stopped before the op's first attempt".to_owned()),
            Err(_) => Err(format!("no first attempt within {within:?}")),
        };
        Ok(match first {
            Ok(()) => Published::Landed,
            Err(reason) => Published::Queued(reason),
        })
    }

    /// Queue `op` and return once its row is stored, behind our earlier ops
    /// on the stream.
    ///
    /// # Errors
    /// As [`publish_answer`](Self::publish_answer).
    pub async fn queue(&self, community: &str, record: &str, op: Vec<u8>) -> Result<(), String> {
        let (entry, client_id) = row(community, record, &op)?;
        self.enqueue(entry, community, &client_id).await
    }

    /// Queue `op` as [`publish_answer`](Self::publish_answer) does, and wait until it
    /// landed on `M`/2 with our `Frontier` on `M`/3, through the outbox's
    /// retries, for at most `within` (plan E3.2 S6 (4)).
    ///
    /// [`Published::Queued`]: the op is not yet published after `within`;
    /// its row stays queued and the outbox lands it later.
    ///
    /// # Errors
    /// As [`publish_answer`](Self::publish_answer).
    pub async fn publish_landed(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
        within: std::time::Duration,
    ) -> Result<Published, String> {
        let (entry, client_id) = row(community, record, &op)?;
        let landed = self.waiters.wait_landed(community, &client_id);
        self.enqueue(entry, community, &client_id).await?;
        let waited = tokio::time::timeout(within, landed).await;
        self.waiters.forget(community, &client_id);
        match waited {
            Ok(Ok(())) => Ok(Published::Landed),
            Ok(Err(_)) => Err("the outbox stopped before the op landed".to_owned()),
            Err(_) => Ok(Published::Queued(format!("not landed within {within:?}"))),
        }
    }

    /// [`publish_landed`](Self::publish_landed) for an invite's
    /// `InviteCreated` (`op`, of `invite_id`): its row and a
    /// revoke-unless-landed row behind it are stored in one transaction, the
    /// second due after `within`. Landed: the revoke row is discarded before
    /// the answer, so the link is handed out only with no revoke behind it.
    /// Otherwise it stays: whatever stops the caller (a crash, the outbox
    /// stopping, a storage error writing the tombstone) the outbox revokes
    /// the invite once its op lands (review RV4 M3).
    ///
    /// # Errors
    /// As [`publish_answer`](Self::publish_answer), or the revoke row was not
    /// discarded (the link is not handed out).
    pub async fn publish_invite_created(
        &self,
        community: &str,
        record: &str,
        op: Vec<u8>,
        invite_id: [u8; 16],
        within: std::time::Duration,
    ) -> Result<Published, String> {
        let (entry, client_id) = row(community, record, &op)?;
        let revoke_id = revoke_client_id(invite_id);
        let revoke = Entry {
            kind: StreamKind::GovernanceOp,
            scope: community.to_owned(),
            client_id: revoke_id.clone(),
            payload: encode_revoke(community, invite_id).ok_or("community key too long")?,
            deadline: Deadline::Horizon,
        };
        let created_row = self.outbox.new_row(entry);
        let mut revoke_row = self.outbox.new_row(revoke);
        let after = i64::try_from(within.as_millis()).unwrap_or(i64::MAX);
        revoke_row.next_at = revoke_row.next_at.saturating_add(after);
        let landed = self.waiters.wait_landed(community, &client_id);
        if let Err(error) = self
            .outbox
            .deps()
            .insert_all(vec![created_row, revoke_row])
            .await
        {
            self.waiters.forget(community, &client_id);
            return Err(error.to_string());
        }
        self.outbox.wake();
        let waited = tokio::time::timeout(within, landed).await;
        self.waiters.forget(community, &client_id);
        match waited {
            Ok(Ok(())) => {
                self.outbox
                    .discard(&revoke_id)
                    .await
                    .map_err(|e| format!("the invite's revoke row stays: {e}"))?;
                Ok(Published::Landed)
            }
            Ok(Err(_)) => Err("the outbox stopped before the op landed".to_owned()),
            Err(_) => Ok(Published::Queued(format!("not landed within {within:?}"))),
        }
    }

    /// Whether the op `client_id` (its hash, hex) is still queued on its
    /// stream: stored, not landed, not failed. Durable across restarts.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn is_queued(&self, client_id: &str) -> Result<bool, String> {
        let row = rekindle_outbox::OutboxDeps::row(self.outbox.deps(), client_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.is_some_and(|r| rekindle_outbox::state::is_unsettled(r.state)))
    }

    /// Our ops `client_ids` (their hashes, hex) of `community` are on our
    /// member record by a write other than their rows' sends: a re-join wrote
    /// our whole chain onto its fresh record (`mirror_own_chain`, ADR 0011's
    /// re-created `M`), and a row still aimed at the record it replaced can
    /// never land there (Veilid takes a DFLT owner subkey only from the owner
    /// key, and the vault's `MemberRecordOwner` is now the fresh record's).
    /// Each such row settles `sent` and leaves the stream, so the ops queued
    /// behind it, the re-join's `MemberJoined` first, go on (D22: the head
    /// blocks the rest). Returns how many rows settled.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn settle_mirrored(
        &self,
        community: &str,
        client_ids: &[String],
    ) -> Result<usize, String> {
        let settled = self
            .outbox
            .settle_landed(client_ids)
            .await
            .map_err(|e| e.to_string())?;
        for row in &settled {
            self.waiters.landed(community, &row.client_id);
        }
        Ok(settled.len())
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
        match decode(&row.payload) {
            Some(RowBody::Op {
                community,
                record,
                op,
            }) => match self
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
            },
            // Its `InviteCreated` landed ahead of it and the link was never
            // handed out (the answer discards this row first): revoke it.
            Some(RowBody::RevokeUnlessLanded {
                community,
                invite_id,
            }) => match self
                .governance
                .revoke_unhanded_invite(community, invite_id)
                .await
            {
                Ok(()) => AttemptOutcome::Sent,
                Err(error) => {
                    tracing::debug!(community, %error, "unhanded invite not revoked yet");
                    AttemptOutcome::Deferred
                }
            },
            None => {
                if let Some(community) = row.stream.split_once(':').map(|(_, scope)| scope) {
                    self.waiters
                        .failed(community, "unreadable governance op row");
                }
                AttemptOutcome::Wedged("unreadable governance op row".into())
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
            Some(super::RowBody::Op {
                community: "VLD0:a",
                record: "VLD0:m",
                op: &[1u8, 2, 3][..],
            })
        );
    }

    #[test]
    fn a_revoke_row_payload_round_trips() {
        let payload = super::encode_revoke("VLD0:a", [7; 16]).unwrap();
        assert_eq!(
            super::decode(&payload),
            Some(super::RowBody::RevokeUnlessLanded {
                community: "VLD0:a",
                invite_id: [7; 16],
            })
        );
        assert!(!super::is_op_row(&super::revoke_client_id([7; 16])));
    }
}
