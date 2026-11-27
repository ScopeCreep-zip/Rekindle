//! Publication policies for freshly signed governance entries.

use rekindle_types::governance::GovernanceEntry;

use super::{unlock_hold, write, WriteEntry};
use crate::deps::{GovernanceRuntimeDeps, OpLanding, OpOutcome, OpPublication};
use crate::error::GovernanceRuntimeError;
use crate::op_log;

/// Sign and install a forum thread through the normal governance pipeline.
/// The caller commits the signed op, device head, and starter together.
///
/// # Errors
/// Validation, the signing barrier, or the atomic commit failed.
pub async fn write_forum_thread<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
    commit: impl FnOnce(op_log::HeldOp) -> ForumCommitFuture + Send + 'static,
) -> Result<(), GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        WriteEntry::ForumThread(Box::new(entry), Box::new(commit)),
        OpLanding::Queued,
        op_log::Deps::Frontier,
    )
    .await
    .map(drop)
}

pub type ForumCommitFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), GovernanceRuntimeError>> + Send>>;
pub(super) type ForumCommit = Box<dyn FnOnce(op_log::HeldOp) -> ForumCommitFuture + Send>;

/// A forum reply requires a successful atomic commit before any projection
/// or gossip. Other governance writes retain their own publication policy.
pub(super) async fn publish_signed<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    held: op_log::HeldOp,
    landing: OpLanding,
    turn: &unlock_hold::SigningTurn,
    commit: Option<ForumCommit>,
) -> Result<OpPublication, GovernanceRuntimeError> {
    let hash = hex::encode(held.hash.0);
    if let Some(commit) = commit {
        // Record the op before the engine can settle it. Roll this
        // in-memory reservation back if the database refuses the commit.
        turn.signed(&hash);
        if let Err(error) = commit(held).await {
            turn.settled(&hash);
            return Err(error);
        }
        return Ok(OpPublication::settled(Ok(OpOutcome::Pending)));
    }
    let publication = deps
        .publish_governance_op(community_id, held.encoded.clone(), landing)
        .await?;
    deps.op_store()
        .store_signed_governance_op(community_id, &held)
        .await?;
    turn.signed(&hash);
    Ok(publication)
}
