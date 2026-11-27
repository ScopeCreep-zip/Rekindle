//! Holding our governance writes behind the unlock barrier (plan E3.2
//! S7d-keys.8). A write signs at once only when the barrier passed with
//! nothing held and our earlier ops landed. Any other write is held: it is
//! listed as "pending: catching up", its request is answered at once
//! (`OpOutcome::Held`, lead decision: a held write never blocks its
//! request), and it is signed in the background in arrival order, each
//! after our previous op's append landed. A restore stops signing: the
//! write stays held, unsigned (S7d-keys.9/.10 take it from there). Every
//! change of the held list is sent in full
//! (`GovernanceRuntimeDeps::emit_catch_up`), and the clear once. A write
//! whose answer needs the op signed (an invite link) is never held: while
//! the barrier catches up it is refused at once; once it passed, it waits
//! in line behind our earlier ops, bounded by its command's deadline.

use std::sync::Arc;

use rekindle_types::display_catch_up::{CatchUpView, PendingGovOp};

use crate::deps::{GovernanceRuntimeDeps, OpLanding};
use crate::error::GovernanceRuntimeError;
use crate::unlock_barrier::{BarrierPhase, UnlockBarrier};

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

/// A forum post answers only once the thread and starter are committed.
/// While another op holds the signing turn, the caller can retry later.
pub(crate) fn take_immediate<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<SigningTurn, GovernanceRuntimeError> {
    let barrier = deps.unlock_barrier(community_id);
    refuse_unheld(&barrier, community_id)?;
    SigningTurn::try_now(barrier)
        .ok_or_else(|| GovernanceRuntimeError::OwnOpsPending(community_id.to_owned()))
}

/// The write's slot in `community_id`: its turn now, or held (built by
/// `held` from its id, and announced). A write whose `landing` is the
/// invite link's is never held: it waits in line for its turn, up to its
/// deadline ([`wait_in_line`]).
///
/// # Errors
/// For a write that may not be held, nothing signed:
/// [`GovernanceRuntimeError::CatchingUp`] while the barrier catches up,
/// [`GovernanceRuntimeError::RestoreDetected`] after a restore, and
/// [`GovernanceRuntimeError::OwnOpsPending`] when its turn did not come by
/// its deadline.
pub(crate) async fn take_slot<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    landing: OpLanding,
    held: impl FnOnce(String) -> PendingGovOp,
) -> Result<BarrierSlot, GovernanceRuntimeError> {
    let barrier = deps.unlock_barrier(community_id);
    if let Some(turn) = SigningTurn::try_now(Arc::clone(&barrier)) {
        return Ok(BarrierSlot::Now(turn));
    }
    let deadline = match landing {
        OpLanding::FirstAttempt | OpLanding::Queued => None,
        OpLanding::InviteLanded { deadline, .. } => Some(deadline),
    };
    if deadline.is_some() {
        refuse_unheld(&barrier, community_id)?;
    }
    let (op_id, ticket) = barrier.hold_next(held);
    announce(deps, community_id, &barrier);
    let write = HeldGovWrite {
        barrier,
        op_id,
        ticket,
    };
    match deadline {
        None => Ok(BarrierSlot::Held(write)),
        Some(deadline) => wait_in_line(deps, community_id, write, deadline).await,
    }
}

/// A write that may not be held (an invite link) is refused at once unless
/// `barrier` passed: its caller checks this before any network step, so
/// the answer never waits on one (plan E3.2 S7d-keys.8).
///
/// # Errors
/// [`GovernanceRuntimeError::CatchingUp`] while the barrier catches up,
/// [`GovernanceRuntimeError::RestoreDetected`] after a restore.
pub(crate) fn refuse_unheld(
    barrier: &UnlockBarrier,
    community_id: &str,
) -> Result<(), GovernanceRuntimeError> {
    match barrier.phase() {
        BarrierPhase::CatchingUp => {
            Err(GovernanceRuntimeError::CatchingUp(community_id.to_owned()))
        }
        BarrierPhase::Restored(fact) => Err(GovernanceRuntimeError::RestoreDetected {
            community: community_id.to_owned(),
            last_seq: fact.last_seq,
        }),
        BarrierPhase::Passed => Ok(()),
    }
}

/// A write that may not be held, past the barrier, waits in line for its
/// turn (behind the writes held before it, once our earlier ops landed)
/// until `deadline`, the command's network deadline, shared with its
/// landing wait. Listed while it waits.
///
/// # Errors
/// [`GovernanceRuntimeError::OwnOpsPending`] at the deadline, or the
/// restore [`HeldGovWrite::turn`] found: the write leaves the list unsigned
/// and its place passes on.
async fn wait_in_line<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    write: HeldGovWrite,
    deadline: tokio::time::Instant,
) -> Result<BarrierSlot, GovernanceRuntimeError> {
    let barrier = Arc::clone(&write.barrier);
    let (op_id, ticket) = (write.op_id.clone(), write.ticket);
    let refused = match tokio::time::timeout_at(deadline, write.turn(deps, community_id)).await {
        Ok(Ok(turn)) => return Ok(BarrierSlot::Now(turn)),
        Ok(Err(refused)) => refused,
        Err(_) => GovernanceRuntimeError::OwnOpsPending(community_id.to_owned()),
    };
    barrier.release(&op_id);
    announce(deps, community_id, &barrier);
    deps.spawn_held_write(
        community_id,
        Box::pin(async move { barrier.pass_over(ticket).await }),
    );
    Err(refused)
}

/// A held write refused when its turn came, nothing signed: its request
/// was answered "held" at once, so the refusal is told to the user now
/// (`GovernanceRuntimeEvent::HeldWriteRefused`), never only logged. A
/// restore is not a refusal: that write stays held and listed, unsigned.
pub(crate) fn report_refused<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    label: Option<String>,
    error: &GovernanceRuntimeError,
) {
    tracing::warn!(community = community_id, %error, "held governance write not made");
    if matches!(error, GovernanceRuntimeError::RestoreDetected { .. }) {
        return;
    }
    deps.emit_event(crate::event::GovernanceRuntimeEvent::HeldWriteRefused {
        community_id: community_id.to_owned(),
        label,
        reason: error.to_string(),
    });
}

impl HeldGovWrite {
    /// A held write takes effect locally at once: a held `InviteRevoked`
    /// (a refused invite's, queued behind its unlanded `InviteCreated`)
    /// stops the invite being listed now, not once seq n landed.
    pub(crate) fn echo<D>(&self, entry: &crate::apply::WriteEntry<D>) {
        use rekindle_types::governance::GovernanceEntry;
        if let crate::apply::WriteEntry::Ready(ready) = entry {
            if let GovernanceEntry::InviteRevoked { invite_id } = **ready {
                self.barrier.withdraw_invite(invite_id);
            }
        }
    }

    /// The words the held list shows the write under ("Kick Bea").
    pub(crate) fn label(&self) -> Option<String> {
        let view = self.barrier.view()?;
        let listed = view.pending.into_iter().find(|p| p.op_id == self.op_id)?;
        Some(listed.label)
    }

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
    fn try_now(barrier: Arc<UnlockBarrier>) -> Option<Self> {
        let order = barrier.try_turn_now()?;
        Some(Self {
            barrier,
            held: None,
            _order: order,
        })
    }

    /// Our op `op_hash` (hex) is signed and queued: the next write waits
    /// for it to land.
    pub(crate) fn signed(&self, op_hash: &str) {
        self.barrier.signed(op_hash);
    }

    /// Undo the signing reservation after a failed commit.
    pub(crate) fn settled(&self, op_hash: &str) {
        self.barrier.settled(op_hash);
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

#[cfg(test)]
#[path = "unlock_hold_tests.rs"]
mod tests;
