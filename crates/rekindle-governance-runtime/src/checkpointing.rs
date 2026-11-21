//! Proposing and endorsing checkpoints (plan E3.2 S7c.4), driven once per
//! `SWEEP_PERIOD` tick by the Host's anti-entropy loop. The rules are pure
//! (`rekindle_governance::checkpoint::{proposal_due, endorsement}`); this
//! module holds the two local clocks they take, builds the body from our
//! replay, seals the directory pages and writes our `Checkpoint` op. No
//! reader checks either clock: a hint is acceleration, never truncation.

use rekindle_governance::checkpoint::{
    beyond, causal_past, eligible_set, endorsement, proposal_due, proposals, Replay, MAX_CUT_HEADS,
};
use rekindle_governance::merge::{merge_cut, merge_outcome};
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_types::governance::{Checkpoint, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::key_format::vld0_key_bytes;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log::{author_record, frontier_refs, verify_held_with};

/// One community's local trigger clocks, kept by the caller between ticks.
#[derive(Debug, Default)]
pub struct CheckpointClock {
    /// The op count at the last tick: unchanged means one `SWEEP_PERIOD`
    /// passed with no newer op.
    ops_seen: Option<usize>,
    /// Our latest endorsement (height, round) and whether it was already
    /// ours at the previous tick (true once we wrote it), so at this tick
    /// it has waited one `SWEEP_PERIOD`.
    waiting: Option<((u64, u32), bool)>,
}

/// What a tick did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointStep {
    Idle,
    Proposed { height: u64 },
    Endorsed { height: u64, round: u32 },
}

/// One tick for `community_id`: endorse a proposal at the height after the
/// selected hint when one is held, else propose that height when due.
///
/// # Errors
/// No cert, no identity, the op store failed, a page write failed, or our
/// op was refused.
pub async fn tick<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    clock: &mut CheckpointClock,
) -> Result<CheckpointStep, GovernanceRuntimeError> {
    // A partial community's op set is not the community's (S7c.8 (1)).
    if crate::partial::is_partial(deps, community_id).await {
        return Ok(CheckpointStep::Idle);
    }
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_owned()))?;
    let secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let key = derive::derive_community_pseudonym(&secret, community_id);
    let me = PseudonymKey(key.verifying_key().to_bytes());
    let community = vld0_key_bytes(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_owned()))?;
    let set = verify_held_with(
        deps,
        community_id,
        &deps.governance_ops(community_id).await?,
    );
    let quiet = clock.ops_seen.replace(set.ops.len()) == Some(set.ops.len());
    let outcome = merge_outcome(&cert, &set.ops);
    let replay = Replay {
        cert: &cert,
        ops: &set.ops,
        community_id: community,
    };
    let prev = outcome.hints.latest();
    let height = prev.map_or(1, |p| p.height + 1);
    let prev_ref: Option<OpRef> = prev.and_then(|p| {
        let op = set.ops.iter().find(|o| o.hash() == p.op)?;
        author_record(&cert, &outcome.state, op.author()).map(|r| op.op_ref(&r))
    });
    if prev.is_some() && prev_ref.is_none() {
        return Ok(CheckpointStep::Idle);
    }

    if !proposals(&set.ops, &outcome, height).is_empty() {
        let timed_out = clock.timed_out(&me, &set.ops, height);
        let Some(body) = endorsement(&me, &set.ops, &outcome, height, timed_out, |c| {
            replay.recomputes(c)
        }) else {
            return Ok(CheckpointStep::Idle);
        };
        let round = body.round;
        crate::apply::write_checkpoint_queued(deps, community_id, body).await?;
        clock.waiting = Some(((height, round), true));
        return Ok(CheckpointStep::Endorsed { height, round });
    }

    let prev_body = prev.map(|p| &p.checkpoint);
    if !proposal_due(beyond(&set.ops, &outcome, prev_body), quiet) {
        return Ok(CheckpointStep::Idle);
    }
    let prev_state = prev_body.map(|p| merge_cut(&cert, &causal_past(&set.ops, &p.cut)).0);
    if !eligible_set(&cert, prev_state.as_ref()).contains(&me) {
        return Ok(CheckpointStep::Idle);
    }
    let mut heads = frontier_refs(&cert, &outcome.state, &set);
    heads.truncate(MAX_CUT_HEADS);
    let Some(mut body) = replay.body(prev_ref.as_ref(), &heads) else {
        return Ok(CheckpointStep::Idle);
    };
    body.pages = seal_pages(deps, &replay, &body).await?;
    let proposed = body.clone();
    crate::apply::write_checkpoint_queued(deps, community_id, body).await?;
    deps.warm_checkpoint_pages(community_id, &proposed);
    clock.waiting = Some(((height, 0), true));
    Ok(CheckpointStep::Proposed { height })
}

impl CheckpointClock {
    /// Whether our latest endorsement at `height` has waited one
    /// `SWEEP_PERIOD`; it then counts as timed out (whether it reached a
    /// majority is `endorsement`'s to check).
    fn timed_out(
        &mut self,
        me: &PseudonymKey,
        ops: &[rekindle_governance::op::VerifiedOp],
        height: u64,
    ) -> bool {
        let latest = (ops.iter())
            .filter(|o| o.author() == me)
            .filter_map(|o| match o.body() {
                rekindle_types::governance::GovernanceEntry::Checkpoint(c)
                    if c.height == height =>
                {
                    Some(c.round)
                }
                _ => None,
            })
            .max();
        let Some(round) = latest else {
            self.waiting = None;
            return false;
        };
        match &mut self.waiting {
            Some((at, waited)) if *at == (height, round) => std::mem::replace(waited, true),
            _ => {
                self.waiting = Some(((height, round), true));
                false
            }
        }
    }
}

/// Seal each directory page of `body` into its own DFLT(1) record, written
/// once with a fresh owner that is then dropped (ADR 0011 "Sealed pages"):
/// the page keys in page order.
async fn seal_pages<D: GovernanceRuntimeDeps>(
    deps: &D,
    replay: &Replay<'_>,
    body: &Checkpoint,
) -> Result<Vec<String>, GovernanceRuntimeError> {
    let dir = replay
        .directory_of(body)
        .ok_or_else(|| GovernanceRuntimeError::Encoding("checkpoint directory".into()))?;
    let mut keys = Vec::with_capacity(dir.page_count());
    for page in (0..dir.page_count()).filter_map(|i| dir.page(replay.community_id, body.height, i))
    {
        let record = deps.create_record(&schema::sealed_page()).await?;
        let outcome = deps
            .set_dht_value(record.lease, 0, page.encode(), record.owner_keypair.clone())
            .await;
        deps.release_record(record.lease).await;
        if let Some(stale) = outcome? {
            return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
        }
        keys.push(record.record_key);
    }
    Ok(keys)
}

#[cfg(test)]
#[path = "checkpointing_tests.rs"]
mod tests;
