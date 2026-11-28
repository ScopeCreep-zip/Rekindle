//! The Host's `governance_checkpoints` rows (plan E3.2 S7c.6): each
//! checkpoint digest's state after a rebuild, and a usable hint's kept
//! snapshot, through `rekindle_db::repo::governance_checkpoints`; and the
//! partial flag of a community joined from a hint (S7c.8 (1)), through
//! `rekindle_db::repo::governance_partial`.

use rekindle_db::repo::governance_checkpoints::{self as repo, CheckpointRow, CheckpointState};
use rekindle_db::repo::governance_partial::{self as partial_repo, PartialRow};
use rekindle_governance_runtime::checkpoint_rows::{CheckpointFact, HintState};
use rekindle_governance_runtime::partial::PartialJoin;
use rekindle_governance_runtime::GovernanceRuntimeError;

use super::HostGovernance;

fn adapter(context: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{context}: {e}"))
}

fn row_of(fact: &CheckpointFact) -> CheckpointRow {
    CheckpointRow {
        height: fact.height,
        round: fact.round,
        digest: fact.digest,
        op_hash: fact.op.0,
        dir_size: fact.dir_size,
        state: match fact.state {
            HintState::Proposed => CheckpointState::Proposed,
            HintState::Usable => CheckpointState::Usable,
            HintState::Equivocated => CheckpointState::Equivocated,
        },
    }
}

impl HostGovernance {
    pub(super) async fn record_governance_checkpoints_impl(
        &self,
        community_id: &str,
        facts: Vec<CheckpointFact>,
    ) -> Result<(), GovernanceRuntimeError> {
        if facts.is_empty() {
            return Ok(());
        }
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        let rows: Vec<CheckpointRow> = facts.iter().map(row_of).collect();
        db.call(move |conn| repo::record(conn, &owner, &community, &rows))
            .await
            .map_err(|e| adapter("governance checkpoints", e))
    }

    pub(super) async fn usable_checkpoint_snapshot_impl(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        db.call(move |conn| repo::usable_snapshot(conn, &owner, &community, height, &digest))
            .await
            .map_err(|e| adapter("checkpoint snapshot", e))
    }

    pub(super) async fn keep_checkpoint_snapshot_impl(
        &self,
        community_id: &str,
        height: u64,
        digest: [u8; 32],
        snapshot: Vec<u8>,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        db.call(move |conn| {
            repo::set_snapshot(conn, &owner, &community, height, &digest, &snapshot).map(drop)
        })
        .await
        .map_err(|e| adapter("keep checkpoint snapshot", e))
    }

    pub(super) async fn governance_partial_impl(
        &self,
        community_id: &str,
    ) -> Result<Option<PartialJoin>, GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        let row = db
            .call(move |conn| partial_repo::get(conn, &owner, &community))
            .await
            .map_err(|e| adapter("governance partial", e))?;
        Ok(row.map(|r| PartialJoin {
            height: r.height,
            digest: r.digest,
            state_root: r.state_root,
        }))
    }

    pub(super) async fn set_governance_partial_impl(
        &self,
        community_id: &str,
        hint: Option<PartialJoin>,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        db.call(move |conn| match hint {
            Some(h) => partial_repo::set(
                conn,
                &owner,
                &community,
                &PartialRow {
                    height: h.height,
                    digest: h.digest,
                    state_root: h.state_root,
                },
            ),
            None => partial_repo::clear(conn, &owner, &community),
        })
        .await
        .map_err(|e| adapter("set governance partial", e))
    }
}
