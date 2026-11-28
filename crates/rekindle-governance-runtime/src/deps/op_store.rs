//! The governance op store (plan E3.2): the held op set, each op's fate,
//! the checkpoint rows and snapshots and the partial-join mark. One
//! responsibility of the runtime's deps, reached through
//! [`GovernanceRuntimeDeps::op_store`](super::GovernanceRuntimeDeps::op_store),
//! as the role table is through `roles`. Writing and publishing an op
//! (gossip, the outbox) stay on the composite trait.

use async_trait::async_trait;

use crate::error::GovernanceRuntimeError;

/// Reads and writes of a community's stored governance ops.
// `automock` must precede `async_trait`, as on `GovernanceRuntimeDeps`.
// The allow covers mockall's *generated* body, which stores expectations
// in `std::sync::Mutex` (architecture rule B9): `cfg(test)` only, so no
// mock code is ever linked into a shipped binary; the full rationale is
// on `GovernanceRuntimeDeps`.
#[cfg_attr(
    test,
    allow(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "mockall's generated mock is test-only code we do not author; see B9 note on GovernanceRuntimeDeps"
    )
)]
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait GovernanceOpStore: Send + Sync {
    /// Every op `governance_ops` holds for the community.
    async fn governance_ops(
        &self,
        community_id: &str,
    ) -> Result<Vec<crate::op_log::HeldOp>, GovernanceRuntimeError>;
    /// Store an op ingested from anywhere but our own signer (gossip, the
    /// DAG fetch, a bootstrap or the barrier's walk; an op already held
    /// keeps its row) and advance its device key's `governance_heads` row.
    /// Never marks the op `signed_here`.
    async fn store_governance_op(
        &self,
        community_id: &str,
        op: &crate::op_log::HeldOp,
    ) -> Result<(), GovernanceRuntimeError>;
    /// Store an op this device just signed, marked `signed_here`: the local
    /// signed log self-detection checks every ingested op of our device key
    /// against (ADR 0022 rule 4, plan S7d-keys.3/.9). Only the local signer
    /// calls it.
    async fn store_signed_governance_op(
        &self,
        community_id: &str,
        op: &crate::op_log::HeldOp,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The highest seq this device signed under `device_key` in the
    /// community (`signed_here` ops only, plan E3.2 S7d-keys.3): the unlock
    /// barrier's local head (S7d-keys.8), never an op of ours re-learnt
    /// from a peer.
    async fn signed_here_head(
        &self,
        community_id: &str,
        device_key: &[u8; 32],
    ) -> Result<Option<u64>, GovernanceRuntimeError>;
    /// Record each op's fate in the last merge.
    async fn set_governance_op_fates(
        &self,
        community_id: &str,
        fates: Vec<crate::op_log::OpVerdict>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// Write each checkpoint digest's state after the last merge
    /// (`governance_checkpoints`, plan E3.2 S7c.6); a kept snapshot stays.
    async fn record_governance_checkpoints(
        &self,
        community_id: &str,
        facts: Vec<crate::checkpoint_rows::CheckpointFact>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The snapshot kept for the checkpoint at `height` with `digest`, only
    /// while its row is usable.
    async fn usable_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    /// Keep `snapshot`, verified against its checkpoint's `stateRoot`, on
    /// that checkpoint's row when the row is usable.
    async fn keep_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
        snapshot: Vec<u8>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The hint `community_id` was joined from while its backfill has not
    /// ended (`governance_partial`, plan E3.2 S7c.8 (1)); `None` once the
    /// community is whole.
    async fn governance_partial(
        &self,
        community_id: &str,
    ) -> Result<Option<crate::partial::PartialJoin>, GovernanceRuntimeError>;
    /// Mark `community_id` partial from `hint`, or whole with `None`.
    async fn set_governance_partial(
        &self,
        community_id: &str,
        hint: Option<crate::partial::PartialJoin>,
    ) -> Result<(), GovernanceRuntimeError>;
    /// The encoded op `op_ref` names, read from its author's op log
    /// (`rekindle_records::chain::read_log_after` on
    /// `op_ref.author_record`/`GOV_HEAD`, chain seq `author_seq + 1`),
    /// through `logs` ([`WalkLogCache::entry`](crate::op_walk::WalkLogCache::entry)):
    /// one read per author log per walk, rebuild or sweep (plan S7c.1).
    async fn fetch_governance_op(
        &self,
        op_ref: &rekindle_types::governance::OpRef,
        logs: &crate::op_walk::WalkLogCache,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
}
