//! The head a departure records (A/B finding R3-8, plan S9): the
//! `MemberRemoved` or our `MemberLeft` carries it, signed, to every reader,
//! so every reader keeps the same of the member's posts from history. A
//! reader computing the head itself at merge time would hold a different
//! value than its peers and keep different posts.
//!
//! A channel has no record (plan E3.3): every post is an entry of its
//! author's channel outbox (`M`/4), so the one head is the outbox's: the
//! record is the member record, the seq is the chain seq of its newest
//! entry and the digest is of that entry's bytes ([`outbox_head`]). The
//! chain is hash-linked and its pages sealed, so the head entry commits to
//! every entry below it, and an entry appended after the departure, or one
//! written into a gap below a head the member signed above its published
//! chain (veilid-core 0.5.8 `set_value` `min_seqnum`), is never at a head
//! every reader reproduces.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_records::chain::{local_newest_entry, ChainEntry};
use rekindle_records::port::RecordPort;
use rekindle_types::dht_layout::member_record::OUTBOX;
use rekindle_types::governance::SlotHead;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// The departure heads of `target_hex`'s channel outbox (`M`/4), as
    /// our local copy holds it. Never a network read: the removal is
    /// authored at the state we hold, and a round trip before it lets ops
    /// that arrive meanwhile into its causal past, so a kick concurrent
    /// with the remover's own removal would no longer be void (netsim
    /// `a_removal_concurrent_with_the_removers_own_is_void_everywhere`).
    /// A channel has no record (plan E3.3): every post is an entry of its
    /// author's outbox, so the outbox head is the one head a departure
    /// records. None when the member never posted, or none of its entries
    /// reached us.
    pub(super) async fn departed_slot_heads_impl(
        &self,
        community_id: &str,
        target_hex: &str,
    ) -> Result<Vec<SlotHead>, GovernanceRuntimeError> {
        let target = (hex::decode(target_hex).ok())
            .and_then(|bytes| bytes.try_into().ok())
            .map(PseudonymKey)
            .ok_or_else(|| adapter(format!("not a pseudonym: {target_hex}")))?;
        let pool = self.transport()?.require_records().map_err(adapter)?;
        let heads: Vec<SlotHead> = (self.outbox_head_of(community_id, &*pool, &target).await)
            .into_iter()
            .collect();
        if heads.is_empty() {
            tracing::info!(
                community_id,
                target = %target_hex,
                "departed member's outbox holds no entry we hold; the departure records no head"
            );
        }
        Ok(heads)
    }

    /// The head of `target`'s channel outbox as our local copy holds it
    /// (its member record from the merged state, kept after it departs).
    async fn outbox_head_of(
        &self,
        community_id: &str,
        pool: &dyn RecordPort,
        target: &PseudonymKey,
    ) -> Option<SlotHead> {
        let state = self.governance_state_impl(community_id)?;
        let record = (state.members.get(target).map(|m| m.member_record.clone()))
            .or_else(|| state.op_log_records.get(target).cloned())?;
        match local_newest_entry(pool, &record, OUTBOX).await {
            Ok(newest) => newest.map(|entry| outbox_head(record, &entry)),
            Err(e) => {
                tracing::warn!(
                    community_id,
                    record,
                    error = %e,
                    "channel outbox head not read for a removal; the member's outbox entries are kept by no reader"
                );
                None
            }
        }
    }
}

/// The departure head of a member's channel outbox: its member record,
/// the chain seq of `newest`, its newest entry, and the digest of that
/// entry's bytes (plan E3.3).
fn outbox_head(member_record: String, newest: &ChainEntry) -> SlotHead {
    let seq = u32::try_from(newest.seq).unwrap_or(u32::MAX);
    SlotHead {
        digest: outbox_head_digest(&member_record, seq, &newest.ciphertext),
        record_key: member_record,
        seq,
    }
}

/// The digest a departure head names of the entry at chain seq `seq` of
/// `member_record`'s channel outbox: the slot head digest over the outbox
/// subkey and the entry's bytes, so a reader reproduces it from the chain.
#[must_use]
pub(crate) fn outbox_head_digest(member_record: &str, seq: u32, entry: &[u8]) -> [u8; 32] {
    rekindle_secrets::slot_head::slot_head_digest(member_record, OUTBOX, seq, entry)
}

fn adapter(e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(e.to_string())
}
