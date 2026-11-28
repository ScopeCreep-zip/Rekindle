//! The daemon's [`GovernanceOpStore`]: the stored governance ops, their
//! fates, the checkpoint rows and snapshots and the partial mark, reached
//! through `GovernanceRuntimeDeps::op_store` as the role table is through
//! `roles`. Delegation only, as `deps_impl.rs`: every body is one call
//! into a sibling module.

use async_trait::async_trait;
use rekindle_governance_runtime::checkpoint_rows::CheckpointFact;
use rekindle_governance_runtime::deps::GovernanceOpStore;
use rekindle_governance_runtime::op_log::{HeldOp, OpVerdict};
use rekindle_governance_runtime::op_walk::WalkLogCache;
use rekindle_governance_runtime::partial::PartialJoin;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::governance::OpRef;

use super::HostGovernance;

#[async_trait]
impl GovernanceOpStore for HostGovernance {
    async fn governance_ops(
        &self,
        community_id: &str,
    ) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
        self.governance_ops_impl(community_id).await
    }

    async fn store_governance_op(
        &self,
        community_id: &str,
        op: &HeldOp,
    ) -> Result<(), GovernanceRuntimeError> {
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        let hold = self
            .context()
            .communities
            .governance_ingest_holds
            .for_community(community_id);
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        let mut queue = hold.lock().await;
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        if let Some(queued) = queue.as_mut() {
            if !queued.iter().any(|held| held.hash == op.hash) {
                queued.push_back(op.clone());
            }
            return Ok(());
        }
        self.store_governance_op_impl(community_id, op, false).await
    }

    async fn store_signed_governance_op(
        &self,
        community_id: &str,
        op: &HeldOp,
    ) -> Result<(), GovernanceRuntimeError> {
        self.store_governance_op_impl(community_id, op, true).await
    }

    async fn signed_here_head(
        &self,
        community_id: &str,
        device_key: &[u8; 32],
    ) -> Result<Option<u64>, GovernanceRuntimeError> {
        self.signed_here_head_impl(community_id, device_key).await
    }

    async fn set_governance_op_fates(
        &self,
        community_id: &str,
        fates: Vec<OpVerdict>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.set_governance_op_fates_impl(community_id, fates).await
    }

    async fn record_governance_checkpoints(
        &self,
        community_id: &str,
        facts: Vec<CheckpointFact>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.record_governance_checkpoints_impl(community_id, facts)
            .await
    }

    async fn usable_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.usable_checkpoint_snapshot_impl(community_id, height, digest)
            .await
    }

    async fn keep_checkpoint_snapshot(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
        snapshot: Vec<u8>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.keep_checkpoint_snapshot_impl(community_id, height, digest, snapshot)
            .await
    }

    async fn governance_partial(
        &self,
        community_id: &str,
    ) -> Result<Option<PartialJoin>, GovernanceRuntimeError> {
        self.governance_partial_impl(community_id).await
    }

    async fn set_governance_partial(
        &self,
        community_id: &str,
        hint: Option<PartialJoin>,
    ) -> Result<(), GovernanceRuntimeError> {
        self.set_governance_partial_impl(community_id, hint).await
    }

    async fn fetch_governance_op(
        &self,
        op_ref: &OpRef,
        logs: &WalkLogCache,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        self.fetch_governance_op_impl(op_ref, logs).await
    }
}
