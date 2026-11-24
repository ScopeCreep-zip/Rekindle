//! The unlock barrier (plan E3.2 S7d-keys.8): at every unlock, before the
//! first governance write per community, the DHT read of our `M(p,d)`/2
//! (`dht_layout::member_record::GOV_HEAD`) and a frontier request to every
//! known ring peer ([`crate::frontier_exchange::request_frontier`]). A
//! round takes the highest seq of our device key `d` over the DHT head and
//! every peer that answered within one app-call timeout ([`judge_round`]).
//! The barrier has two exits: a round in which at least one known ring
//! peer answered, or an empty ring (a sole creator, which completes on the
//! DHT head alone). A round with no answer is retried on every ring
//! change, every `HeadAdvanced` and once per app-call timeout, without a
//! time limit (a deadline that completed on the DHT read alone would
//! reopen the gossip-only fork ADR 0022 rule 4 closes).
//!
//! Governance writes wait meanwhile ([`UnlockBarrier::passed`]) and are
//! listed as "pending: catching up" ([`UnlockBarrier::view`]). A result
//! above the local head is a restore ([`RestoreDetected`]): its `last_seq`
//! is the barrier's maximum, not the DHT head alone (IC4), and nothing is
//! signed under `d` again (S7d-keys.10 re-enrols the device).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_types::display_catch_up::{CatchUpView, PendingGovOp, PendingGovState};
use rekindle_types::governance::Frontier;
use tokio::sync::watch;

/// The highest seq of device key `device` among `frontier`'s heads.
#[must_use]
pub fn highest_seq_of(frontier: &Frontier, device: &[u8; 32]) -> Option<u64> {
    frontier
        .heads
        .iter()
        .filter(|head| head.device_key == *device)
        .map(|head| head.author_seq)
        .max()
}

/// One round's inputs: our local head of `d` (the op store's), the DHT
/// `GovHead`, whether any ring peer is known, and the highest seq of `d`
/// each answering peer reported (`None`: the peer knows no op of `d`).
#[derive(Debug, Clone, Default)]
pub struct BarrierRound {
    pub local_head: Option<u64>,
    pub dht_head: Option<u64>,
    pub ring_known: bool,
    pub answers: Vec<Option<u64>>,
}

/// Where the barrier stands for one community.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierPhase {
    /// No round completed yet: governance writes wait.
    CatchingUp,
    /// Every result reads at or below the local head: writes go ahead.
    Passed,
    /// A result above the local head: an older copy of this data root
    /// wrote under `d`. Nothing is signed under `d` again.
    Restored(RestoreDetected),
}

/// The restore fact (S7d-keys.8): the barrier's maximum seq of `d`, which
/// S7d-keys.10's `DeviceRetired.lastSeq` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreDetected {
    pub last_seq: u64,
    pub local_head: Option<u64>,
}

/// Judge one round: `None` when known ring peers exist and none answered
/// (the round is retried), else `Passed` or `Restored` at the maximum over
/// the DHT head and every answer. One stale peer reporting lower never
/// lowers the result.
#[must_use]
pub fn judge_round(round: &BarrierRound) -> Option<BarrierPhase> {
    if round.ring_known && round.answers.is_empty() {
        return None;
    }
    let max = round
        .answers
        .iter()
        .copied()
        .chain(std::iter::once(round.dht_head))
        .flatten()
        .max();
    Some(match (max, round.local_head) {
        (Some(last_seq), local) if local.is_none_or(|local| last_seq > local) => {
            BarrierPhase::Restored(RestoreDetected {
                last_seq,
                local_head: local,
            })
        }
        _ => BarrierPhase::Passed,
    })
}

/// One community's barrier: its phase, the governance writes it holds,
/// the signing turn and our signed ops not yet landed.
#[derive(Debug)]
pub struct UnlockBarrier {
    phase: watch::Sender<BarrierPhase>,
    pending: Mutex<Vec<PendingGovOp>>,
    /// One write signs at a time, in arrival order (tokio's mutex is
    /// fair): seq n + 1 is signed after seq n.
    turn: Arc<tokio::sync::Mutex<()>>,
    /// Our signed ops (hash hex) whose append to `M(p,d)` has not landed;
    /// the next write signs only once it is empty (S7d-keys.8: "seq n + 1
    /// is signed only after seq n's append landed").
    unlanded: watch::Sender<BTreeSet<String>>,
    next_id: AtomicU64,
}

impl Default for UnlockBarrier {
    fn default() -> Self {
        Self {
            phase: watch::Sender::new(BarrierPhase::CatchingUp),
            pending: Mutex::new(Vec::new()),
            turn: Arc::default(),
            unlanded: watch::Sender::new(BTreeSet::new()),
            next_id: AtomicU64::new(0),
        }
    }
}

impl UnlockBarrier {
    /// A fresh id for a held write, stable while it is held.
    #[must_use]
    pub fn next_op_id(&self) -> String {
        format!("held-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Wait for the signing turn (arrival order).
    pub async fn turn(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.turn).lock_owned().await
    }

    /// Our op `op` (hash hex) was signed and queued: the next write waits
    /// for its append to land.
    pub fn signed(&self, op: &str) {
        self.unlanded.send_modify(|set| {
            set.insert(op.to_owned());
        });
    }

    /// Our op `op` landed on our member record, or failed for good: it no
    /// longer holds the next write back.
    pub fn settled(&self, op: &str) {
        self.unlanded.send_if_modified(|set| set.remove(op));
    }

    /// Wait until every op we signed landed (or failed for good).
    pub async fn landed(&self) {
        let mut rx = self.unlanded.subscribe();
        // `wait_for` fails only once the sender drops, and it lives in
        // `self`, borrowed for this wait.
        let _landed = rx.wait_for(BTreeSet::is_empty).await;
    }

    #[must_use]
    pub fn phase(&self) -> BarrierPhase {
        *self.phase.borrow()
    }

    /// Record a round's verdict; `None` (no answer) keeps catching up. A
    /// restore is final for this unlock.
    pub fn complete(&self, verdict: Option<BarrierPhase>) {
        let Some(verdict) = verdict else { return };
        self.phase.send_if_modified(|phase| {
            if matches!(phase, BarrierPhase::Restored(_)) || *phase == verdict {
                return false;
            }
            *phase = verdict;
            true
        });
    }

    /// Hold `op` while the barrier catches up (the full list is the view).
    pub fn hold(&self, op: PendingGovOp) {
        self.pending.lock().push(op);
    }

    /// Mark a held write as being signed (the barrier passed for it).
    pub fn signing(&self, op_id: &str) {
        for op in self
            .pending
            .lock()
            .iter_mut()
            .filter(|op| op.op_id == op_id)
        {
            op.state = PendingGovState::Signing;
        }
    }

    /// Drop a held write once it is signed and handed to the outbox.
    pub fn release(&self, op_id: &str) {
        self.pending.lock().retain(|op| op.op_id != op_id);
    }

    /// The window's view: catching up while no round completed, and every
    /// held write; `None` once passed with nothing held.
    #[must_use]
    pub fn view(&self) -> Option<CatchUpView> {
        let pending = self.pending.lock().clone();
        let catching_up = !matches!(self.phase(), BarrierPhase::Passed);
        (catching_up || !pending.is_empty()).then_some(CatchUpView {
            catching_up,
            pending,
        })
    }

    /// Wait until the barrier passed: `Ok` to sign, `Err` with the restore
    /// fact when this copy must not sign under `d`.
    ///
    /// # Errors
    /// The barrier detected a restore.
    pub async fn passed(&self) -> Result<(), RestoreDetected> {
        let mut rx = self.phase.subscribe();
        loop {
            match *rx.borrow_and_update() {
                BarrierPhase::Passed => return Ok(()),
                BarrierPhase::Restored(fact) => return Err(fact),
                BarrierPhase::CatchingUp => {}
            }
            // `changed` fails only once the sender drops, and it lives in
            // `self`, borrowed for this wait.
            let _changed = rx.changed().await;
        }
    }
}

/// Every community's barrier for this unlock (a lock drops them all).
#[derive(Debug, Default)]
pub struct UnlockBarriers(Mutex<BTreeMap<String, Arc<UnlockBarrier>>>);

impl UnlockBarriers {
    /// `community`'s barrier, created catching up on first use.
    #[must_use]
    pub fn of(&self, community: &str) -> Arc<UnlockBarrier> {
        Arc::clone(self.0.lock().entry(community.to_owned()).or_default())
    }

    /// The window's view of `community`'s barrier, if one is held.
    #[must_use]
    pub fn view(&self, community: &str) -> Option<CatchUpView> {
        self.0.lock().get(community).and_then(|b| b.view())
    }

    /// Forget every barrier (lock): the next unlock starts catching up.
    pub fn clear(&self) {
        self.0.lock().clear();
    }
}

#[cfg(test)]
#[path = "unlock_barrier_tests.rs"]
mod tests;
