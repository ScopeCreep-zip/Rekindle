//! The governance op store and op log on the Host (plan E3.2, ADR 0011):
//! records created with a fresh owner and pinned by `owner_of`, the pinned
//! `GenesisCert`, the op set in `governance_ops`/`governance_heads` (one
//! repo for both tracks, D1), an op fetched by its `OpRef` from its
//! author's `M`/2 chain, and our op queued on the governance-op outbox
//! stream.

use std::collections::BTreeMap;

use rekindle_db::repo::governance_ops::{self as repo, AuthorHead, OpState, StoredOp};
use rekindle_governance_runtime::deps::DhtRecordInfo;
use rekindle_governance_runtime::op_log::{self, HeldOp, OpFate, OpVerdict};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::RecordSchema;
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::governance::{GenesisCert, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

fn adapter(context: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{context}: {e}"))
}

fn state_of(fate: OpFate) -> OpState {
    match fate {
        OpFate::Applied => OpState::Applied,
        OpFate::PendingDeps => OpState::PendingDeps,
        OpFate::Quarantined => OpState::Quarantined,
        OpFate::Equivocation => OpState::Equivocation,
    }
}

fn fate_of(state: OpState) -> OpFate {
    match state {
        OpState::Applied => OpFate::Applied,
        OpState::PendingDeps => OpFate::PendingDeps,
        OpState::Quarantined => OpFate::Quarantined,
        OpState::Equivocation => OpFate::Equivocation,
    }
}

pub(super) fn pseudonym_of(hex_key: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_key).ok()?;
    Some(PseudonymKey(<[u8; 32]>::try_from(bytes).ok()?))
}

/// The held form of the stored `ops`, each with the member record its
/// author's head names.
///
/// # Errors
/// An op's author has no head or is not a pseudonym (the store writes the
/// head with the op, so either is a corrupt store).
pub(crate) fn held_ops(ops: Vec<StoredOp>, heads: &[AuthorHead]) -> Result<Vec<HeldOp>, String> {
    let records: BTreeMap<&str, &str> = heads
        .iter()
        .map(|h| (h.author.as_str(), h.member_record.as_str()))
        .collect();
    ops.into_iter()
        .map(|op| {
            let record = records
                .get(op.author.as_str())
                .ok_or_else(|| format!("op author {} has no head", op.author))?;
            let author = pseudonym_of(&op.author)
                .ok_or_else(|| format!("op author {:?} is not a pseudonym", op.author))?;
            Ok(HeldOp {
                hash: OpHash(op.op_hash),
                author,
                author_record: (*record).to_owned(),
                seq: op.author_seq,
                lamport: op.lamport,
                issued_at: op.issued_at,
                encoded: op.op,
                fate: fate_of(op.state),
            })
        })
        .collect()
}

impl HostGovernance {
    fn op_store(&self) -> Result<(String, rekindle_db::Db), GovernanceRuntimeError> {
        self.owner_and_db()
            .ok_or_else(|| GovernanceRuntimeError::Adapter("no identity is unlocked".into()))
    }

    pub(super) async fn create_record_impl(
        &self,
        schema: &RecordSchema,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let pool = self
            .transport()?
            .require_records()
            .map_err(|e| adapter("record pool", e))?;
        let (lease, record_key, owner) = pool
            .create(schema, None)
            .await
            .map_err(|e| adapter("create record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair: Some(owner),
            lease,
        })
    }

    pub(super) fn owner_of_impl(&self, lease: LeaseId) -> Result<String, GovernanceRuntimeError> {
        self.transport()?
            .require_records()
            .map_err(|e| adapter("record pool", e))?
            .owner_of(lease)
            .map_err(|e| adapter("owner of record", e))
    }

    pub(super) fn genesis_cert_impl(&self, community_id: &str) -> Option<GenesisCert> {
        self.ctx.communities.genesis_cert(community_id)
    }

    pub(super) async fn governance_ops_impl(
        &self,
        community_id: &str,
    ) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        let (ops, heads): (Vec<StoredOp>, Vec<AuthorHead>) = db
            .call(move |conn| {
                Ok((
                    repo::list(conn, &owner, &community, None)?,
                    repo::heads(conn, &owner, &community)?,
                ))
            })
            .await
            .map_err(|e| adapter("governance ops", e))?;
        held_ops(ops, &heads).map_err(GovernanceRuntimeError::Adapter)
    }

    /// Store `op` and advance its author's head when it is newer.
    pub(super) async fn store_governance_op_impl(
        &self,
        community_id: &str,
        op: &HeldOp,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        let author = hex::encode(op.author.0);
        let stored = StoredOp {
            op_hash: op.hash.0,
            author: author.clone(),
            author_seq: op.seq,
            lamport: op.lamport,
            issued_at: op.issued_at,
            op: op.encoded.clone(),
            state: state_of(op.fate),
        };
        let head = AuthorHead {
            author,
            member_record: op.author_record.clone(),
            head_seq: op.seq,
            frontier_hash: op.hash.0,
        };
        db.call(move |conn| {
            let tx = conn.unchecked_transaction()?;
            repo::insert(&tx, &owner, &community, &stored)?;
            let newer = repo::heads(&tx, &owner, &community)?
                .iter()
                .find(|h| h.author == head.author)
                .is_none_or(|h| h.head_seq < head.head_seq);
            if newer {
                repo::set_head(&tx, &owner, &community, &head)?;
            }
            tx.commit()
        })
        .await
        .map_err(|e| adapter("store governance op", e))
    }

    pub(super) async fn set_governance_op_fates_impl(
        &self,
        community_id: &str,
        fates: Vec<OpVerdict>,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        db.call(move |conn| {
            let tx = conn.unchecked_transaction()?;
            for verdict in &fates {
                repo::set_state(
                    &tx,
                    &owner,
                    &community,
                    &verdict.hash.0,
                    state_of(verdict.fate),
                    verdict.lamport,
                )?;
            }
            tx.commit()
        })
        .await
        .map_err(|e| adapter("governance op fates", e))
    }

    /// The op at `op_ref.author_seq` of its author's op log: chain seq
    /// `author_seq + 1` of `M`/2.
    pub(super) async fn fetch_governance_op_impl(
        &self,
        op_ref: &OpRef,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let pool = self
            .transport()?
            .require_records()
            .map_err(|e| adapter("record pool", e))?;
        let read = rekindle_records::chain::read_log_after(
            &*pool,
            &op_ref.author_record,
            GOV_HEAD,
            op_ref.author_seq,
        )
        .await
        .map_err(|e| adapter("read op log", e))?;
        Ok(read
            .entries
            .into_iter()
            .find(|entry| entry.seq == op_ref.author_seq + 1)
            .map(|entry| entry.ciphertext))
    }

    /// Publish `encoded` on our member record's op log (the cert's record
    /// for the creator, else our `MemberJoined`'s) and wait for what
    /// `landing` names: its first attempt (a queued op stands), or its
    /// landing on `M`/2 and `M`/3 within the command's deadline.
    pub(super) async fn publish_governance_op_impl(
        &self,
        community_id: &str,
        encoded: Vec<u8>,
        landing: rekindle_governance_runtime::OpLanding,
    ) -> Result<(), GovernanceRuntimeError> {
        let (cert, state, mine) = {
            let map = self.ctx.communities.map().read();
            let community = map.get(community_id).ok_or_else(|| {
                GovernanceRuntimeError::CommunityNotFound(community_id.to_owned())
            })?;
            (
                community.genesis_cert.clone(),
                community.governance_state.clone().unwrap_or_default(),
                community.my_pseudonym_key.as_deref().and_then(pseudonym_of),
            )
        };
        let cert = cert
            .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
        let mine = mine.ok_or_else(|| adapter("publish op", "no pseudonym"))?;
        let record = op_log::author_record(&cert, &state, &mine)
            .ok_or_else(|| GovernanceRuntimeError::MemberRecordMissing(hex::encode(mine.0)))?;
        let ops = self
            .ctx
            .outbox
            .governance_ops()
            .ok_or_else(|| adapter("publish op", "the outbox is not running"))?;
        match landing {
            rekindle_governance_runtime::OpLanding::FirstAttempt => {
                let published = ops
                    .publish(community_id, &record, encoded.clone())
                    .await
                    .map_err(|e| adapter("publish op", e))?;
                // A first attempt that did not land leaves the op queued: it
                // is in our state and the outbox lands it, so the action stands.
                if let crate::host::adapters::outbox::Published::Queued(reason) = published {
                    tracing::warn!(community = %community_id, %reason, "governance op queued for retry, not yet on the DHT");
                }
            }
            // Bounded by the command's deadline (the client's
            // `NETWORK_COMMAND_DEADLINE`, answered before it).
            rekindle_governance_runtime::OpLanding::Landed => ops
                .publish_landed(
                    community_id,
                    &record,
                    encoded.clone(),
                    rekindle_types::config::NETWORK_ANSWER_DEADLINE,
                )
                .await
                .map_err(|e| adapter("publish op", e))?,
        }
        // The fast path: every member hears the op before its watch fires.
        self.send_to_mesh_impl(
            community_id,
            &rekindle_codec::community::envelope::CommunityEnvelope::Control(
                rekindle_codec::community::envelope::ControlPayload::GovernanceOp { op: encoded },
            ),
        );
        Ok(())
    }

    /// A gossiped `governanceOp @74`: into the op store and the rebuild
    /// (`op_log::receive_op`), for both tracks.
    pub async fn on_governance_op(&self, community_id: &str, op: Vec<u8>) {
        if !self.ctx.communities.map().read().contains_key(community_id) {
            return;
        }
        if let Err(error) = op_log::receive_op(self, community_id, op).await {
            tracing::debug!(community = community_id, %error, "gossiped governance op not stored");
        }
    }

    /// The `governance_op` stream's send: append `op` to `record`'s op log
    /// with `K_M(p)` from the vault.
    pub(crate) async fn append_own_op(
        &self,
        community_id: &str,
        record: &str,
        op: Vec<u8>,
    ) -> Result<(), String> {
        let writer = {
            let guard = self.ctx.vault.lock();
            let vault = guard.as_ref().ok_or("no identity is unlocked")?;
            let key = rekindle_vault::VaultKey::MemberRecordOwner {
                community: community_id.to_owned(),
            };
            let bytes = vault
                .get(&key)
                .map_err(|e| format!("member record owner: {e}"))?
                .ok_or("no member record owner is stored")?;
            zeroize::Zeroizing::new(
                String::from_utf8(bytes.to_vec()).map_err(|_| "member record owner is not text")?,
            )
        };
        let pool = self
            .transport()
            .map_err(|e| e.to_string())?
            .require_records()
            .map_err(|e| format!("record pool: {e}"))?;
        append_op_once(&*pool, record, &writer, op).await?;
        self.write_own_frontier(community_id, record, &writer).await
    }

    /// Write our `Frontier` (`M`/3) after an append: the heads of the op set
    /// we hold, which a joiner's walk starts from.
    async fn write_own_frontier(
        &self,
        community_id: &str,
        record: &str,
        writer: &str,
    ) -> Result<(), String> {
        use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
        let (cert, state) = {
            let map = self.ctx.communities.map().read();
            let community = map.get(community_id).ok_or("community not found")?;
            (
                community
                    .genesis_cert
                    .clone()
                    .ok_or("no genesis cert is pinned")?,
                community.governance_state.clone().unwrap_or_default(),
            )
        };
        let held = self
            .governance_ops(community_id)
            .await
            .map_err(|e| format!("read op store: {e}"))?;
        let set = op_log::verify_held(community_id, &held);
        let heads = op_log::frontier_refs(&cert, &state, &set);
        let bytes = rekindle_codec::governance_op::encode_frontier(&heads)
            .map_err(|e| format!("frontier: {e}"))?;
        let lease = self
            .acquire_record(record, Some(writer.to_owned()))
            .await
            .map_err(|e| format!("open member record: {e}"))?;
        let written = self
            .set_dht_value(
                lease,
                rekindle_types::dht_layout::member_record::FRONTIER,
                bytes,
                Some(writer.to_owned()),
            )
            .await;
        self.release_record(lease).await;
        written
            .map(drop)
            .map_err(|e| format!("write frontier: {e}"))
    }
}

/// Append the encoded `op` to `record`'s op log (`M`/2) once: an earlier
/// attempt whose write landed but whose acknowledgement was lost left the op
/// at chain seq `op.seq + 1` or later, so the log after `op.seq` is read
/// first and an entry with this op's hash settles the attempt without a
/// second append. `true`: this call appended it.
///
/// # Errors
/// The op does not decode, or the read or the append failed.
pub(crate) async fn append_op_once(
    port: &dyn rekindle_records::port::RecordPort,
    record: &str,
    writer: &str,
    op: Vec<u8>,
) -> Result<bool, String> {
    let decoded =
        rekindle_codec::governance_op::decode_op(&op).map_err(|e| format!("governance op: {e}"))?;
    let hash = rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig);
    let recent = rekindle_records::chain::read_log_after(port, record, GOV_HEAD, decoded.op.seq)
        .await
        .map_err(|e| format!("read op log: {e}"))?;
    let logged = recent.entries.iter().any(|entry| {
        rekindle_codec::governance_op::decode_op(&entry.ciphertext)
            .is_ok_and(|d| rekindle_governance::op::op_hash(&d.unsigned, &d.op.sig) == hash)
    });
    if logged {
        return Ok(false);
    }
    rekindle_records::chain::append_at(port, record, GOV_HEAD, writer, &[op])
        .await
        .map(|_| true)
        .map_err(|e| format!("append op: {e}"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
