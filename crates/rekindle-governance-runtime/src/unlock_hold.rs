//! Holding our governance writes behind the unlock barrier (plan E3.2
//! S7d-keys.8). A write signs at once only when the barrier passed with
//! nothing held and our earlier ops landed. Any other write is held: it is
//! listed as "pending: catching up", its request is answered at once
//! (`OpOutcome::Held`, lead decision: a held write never blocks its
//! request), and it is signed in the background in arrival order, each
//! after our previous op's append landed. A restore stops signing: the
//! write stays held, unsigned (S7d-keys.9/.10 take it from there). Every
//! change of the held list is sent in full
//! (`GovernanceRuntimeDeps::emit_catch_up`), and the clear once.

use std::sync::Arc;

use rekindle_types::display_catch_up::{CatchUpView, PendingGovOp};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::unlock_barrier::UnlockBarrier;

/// A write's signing turn: held until the write signed (or refused).
pub(crate) struct SigningTurn {
    barrier: Arc<UnlockBarrier>,
    held: Option<String>,
    _order: tokio::sync::OwnedMutexGuard<()>,
}

/// A held write's place in the queue.
pub(crate) struct HeldGovWrite {
    barrier: Arc<UnlockBarrier>,
    op_id: String,
    ticket: u64,
}

/// Where a write stands at the barrier.
pub(crate) enum BarrierSlot {
    /// Signs now.
    Now(SigningTurn),
    /// Held: answered at once, signed in the background.
    Held(HeldGovWrite),
}

/// Send the barrier's full view, or the clear when nothing is held.
fn announce<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str, barrier: &UnlockBarrier) {
    let view = barrier.view().unwrap_or(CatchUpView {
        catching_up: false,
        pending: Vec::new(),
    });
    deps.emit_catch_up(community_id, view);
}

/// The write's slot in `community_id`: its turn now, or held (built by
/// `held` from its id, and announced).
///
/// # Errors
/// [`GovernanceRuntimeError::CatchingUp`] for a write that may not be held
/// (`may_hold` false: its answer needs the op signed), nothing signed.
pub(crate) fn take_slot<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    may_hold: bool,
    held: impl FnOnce(String) -> PendingGovOp,
) -> Result<BarrierSlot, GovernanceRuntimeError> {
    let barrier = deps.unlock_barrier(community_id);
    if let Some(turn) = barrier.try_turn_now() {
        return Ok(BarrierSlot::Now(SigningTurn {
            barrier,
            held: None,
            _order: turn,
        }));
    }
    if !may_hold {
        return Err(GovernanceRuntimeError::CatchingUp(community_id.to_owned()));
    }
    let (op_id, ticket) = barrier.hold_next(held);
    announce(deps, community_id, &barrier);
    Ok(BarrierSlot::Held(HeldGovWrite {
        barrier,
        op_id,
        ticket,
    }))
}

impl HeldGovWrite {
    /// The id the held list shows the write under.
    pub(crate) fn op_id(&self) -> &str {
        &self.op_id
    }

    /// Wait for the write's turn: after every write held before it, once
    /// the barrier passed and our earlier ops landed.
    ///
    /// # Errors
    /// [`GovernanceRuntimeError::RestoreDetected`]: the barrier found our
    /// device key ahead of this copy; the write stays held, unsigned.
    pub(crate) async fn turn<D: GovernanceRuntimeDeps>(
        self,
        deps: &D,
        community_id: &str,
    ) -> Result<SigningTurn, GovernanceRuntimeError> {
        let barrier = self.barrier;
        barrier.serve(self.ticket).await;
        let turn = barrier.turn().await;
        barrier.served(self.ticket);
        if let Err(fact) = barrier.passed().await {
            return Err(GovernanceRuntimeError::RestoreDetected {
                community: community_id.to_owned(),
                last_seq: fact.last_seq,
            });
        }
        barrier.landed().await;
        barrier.signing(&self.op_id);
        announce(deps, community_id, &barrier);
        Ok(SigningTurn {
            barrier,
            held: Some(self.op_id),
            _order: turn,
        })
    }
}

impl SigningTurn {
    /// Our op `op_hash` (hex) is signed and queued: the next write waits
    /// for it to land.
    pub(crate) fn signed(&self, op_hash: &str) {
        self.barrier.signed(op_hash);
    }

    /// The write is done (signed, or refused before signing): drop it from
    /// the held list and pass the turn on.
    pub(crate) fn finish<D: GovernanceRuntimeDeps>(self, deps: &D, community_id: &str) {
        if let Some(id) = &self.held {
            self.barrier.release(id);
            announce(deps, community_id, &self.barrier);
        }
    }
}
