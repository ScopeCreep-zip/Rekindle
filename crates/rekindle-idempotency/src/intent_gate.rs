//! The intent gate (plan U1.3, ADR 0019; `realignment/r9-intents.md`
//! §7.6): holds every action that is not Routine until a commit carrying
//! the proof its class demands.
//!
//! - [`IntentGate::begin`] runs a Routine action at once and holds any
//!   other, answering with the [`IntentReview`] every frontend renders.
//! - [`IntentGate::commit`] re-derives the review from facts computed
//!   again at commit time. If the class rose or a fact changed, it answers
//!   with a fresh review under a new id and runs nothing ("ask them more
//!   questions … before returning", GOV.UK); otherwise it checks the proof
//!   and hands back the action exactly as reviewed, once.
//! - [`IntentGate::commit_and_run`] runs a committed action once: a
//!   repeated commit of the same id (a double click) gets the first run's
//!   result (plan U1.9).
//! - A wrong passphrase counts against the owner's identity in the shared
//!   [`AttemptLimiter`]; a locked identity's commit is refused before the
//!   passphrase is checked.
//! - A held action belongs to the owner that began it (the host's
//!   connection and identity), expires after [`INTENT_TTL_MS`], and at
//!   most [`MAX_PENDING_PER_OWNER`] are held per owner. Lock, logout and
//!   identity switch [`IntentGate::clear`] the owner's.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use rekindle_types::intent::{
    effective_class, Escalation, Fact, IntentClass, IntentId, IntentKind, IntentProof,
    IntentReview, Notifies, ProofRequired, Reversibility, Subject,
};

use crate::attempts::AttemptLimiter;
use crate::cache::IdempotencyCache;

/// How long a held action waits for its commit.
pub const INTENT_TTL_MS: u64 = 5 * 60 * 1000;
/// Held actions per owner.
pub const MAX_PENDING_PER_OWNER: usize = 16;

/// What the backend knows about a selected action when it is selected,
/// and again at commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub kind: IntentKind,
    pub subject: Subject,
    pub facts: Vec<Fact>,
    pub escalations: Vec<Escalation>,
    pub reversibility: Reversibility,
    pub notifies: Notifies,
}

/// The answer to a selection.
#[derive(Debug)]
pub enum Gate<A> {
    /// Routine (or self-proving): run it now.
    Run(A),
    /// Held: render this review.
    Review(IntentReview),
}

/// The answer to a commit that ran or was sent back for review.
#[derive(Debug, Clone)]
pub enum Committed<R> {
    /// The action ran; this is its result (the same for a repeated commit).
    Ran(R),
    /// The facts changed: render this review; nothing ran.
    Review(IntentReview),
}

/// Why a commit or selection was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    #[error("no held action with this id (expired, cancelled, already run or not ours)")]
    UnknownIntent,
    #[error("the proof does not match what the review asked for")]
    WrongProof,
    #[error("too many wrong passphrases; try again in {retry_after_ms} ms")]
    Locked { retry_after_ms: u64 },
    #[error("too many actions are waiting for review; finish or cancel one")]
    TooManyPending,
}

/// Who holds an action: the gate binds it to the whole owner (connection
/// and identity) and counts wrong passphrases per identity.
pub trait GateOwner: Clone + PartialEq + Send + Sync + 'static {
    type Identity: Clone + Eq + Hash + Send + Sync + 'static;
    fn identity(&self) -> &Self::Identity;
}

/// Why a commit did not run, inside the shared run of `commit_and_run`.
enum NotRun {
    Review(IntentReview),
    Refused(GateError),
}

struct Pending<A, O> {
    owner: O,
    selection: Selection,
    review: IntentReview,
    action: A,
}

/// Held actions of type `A`, owned by `O`, whose runs answer `R`.
pub struct IntentGate<A, O: GateOwner, R: Clone + Send + Sync + 'static> {
    pending: parking_lot::Mutex<HashMap<IntentId, Pending<A, O>>>,
    limiter: Arc<AttemptLimiter<O::Identity>>,
    /// Each run's owner and result, for a repeated commit of its id.
    results: IdempotencyCache<(O, R)>,
}

fn review_of(selection: &Selection, now_ms: u64) -> IntentReview {
    let class = effective_class(selection.kind, &selection.escalations);
    IntentReview {
        intent_id: IntentId::new(),
        kind: selection.kind,
        class,
        subject: selection.subject.clone(),
        facts: selection.facts.clone(),
        escalations: selection.escalations.clone(),
        reversibility: selection.reversibility,
        notifies: selection.notifies,
        proof: class.proof(),
        expires_at_ms: now_ms + INTENT_TTL_MS,
    }
}

impl<A, O: GateOwner, R: Clone + Send + Sync + 'static> IntentGate<A, O, R> {
    /// A gate whose passphrase failures count in `limiter` (shared with
    /// the host's unlock).
    #[must_use]
    pub fn new(limiter: Arc<AttemptLimiter<O::Identity>>) -> Self {
        Self {
            pending: parking_lot::Mutex::new(HashMap::new()),
            limiter,
            results: IdempotencyCache::new(4096, Duration::from_millis(INTENT_TTL_MS)),
        }
    }

    /// Step 1, Select: run `action` now if it is Routine after escalations
    /// (or proves itself), else hold it for `owner` and return its review.
    ///
    /// # Errors
    /// `owner` already holds [`MAX_PENDING_PER_OWNER`] unexpired actions.
    pub fn begin(
        &self,
        owner: O,
        selection: Selection,
        action: A,
        now_ms: u64,
    ) -> Result<Gate<A>, GateError> {
        let class = effective_class(selection.kind, &selection.escalations);
        if class == IntentClass::Routine || selection.kind.proves_itself() {
            return Ok(Gate::Run(action));
        }
        let mut pending = self.pending.lock();
        pending.retain(|_, p| p.review.expires_at_ms > now_ms);
        if pending.values().filter(|p| p.owner == owner).count() >= MAX_PENDING_PER_OWNER {
            return Err(GateError::TooManyPending);
        }
        let review = review_of(&selection, now_ms);
        pending.insert(
            review.intent_id,
            Pending {
                owner,
                selection,
                review: review.clone(),
                action,
            },
        );
        Ok(Gate::Review(review))
    }

    /// Step 3, Commit: with `fresh` = the selection recomputed now, either
    /// a fresh review (the class rose or a fact changed; nothing runs) or
    /// the held action, removed so it runs once. `passphrase_ok` verifies
    /// a passphrase proof against the vault.
    ///
    /// # Errors
    /// Unknown, expired or someone else's id; or a proof of the wrong
    /// shape, a mistyped name or a wrong passphrase (the action stays held).
    pub fn commit(
        &self,
        intent_id: IntentId,
        owner: &O,
        proof: &IntentProof,
        fresh: Selection,
        passphrase_ok: impl FnOnce(&str) -> bool,
        now_ms: u64,
    ) -> Result<Gate<A>, GateError> {
        let mut pending = self.pending.lock();
        let held = pending
            .get(&intent_id)
            .filter(|p| p.owner == *owner && p.review.expires_at_ms > now_ms)
            .ok_or(GateError::UnknownIntent)?;
        if fresh != held.selection {
            let Some(mut held) = pending.remove(&intent_id) else {
                return Err(GateError::UnknownIntent);
            };
            held.review = review_of(&fresh, now_ms);
            held.selection = fresh;
            let review = held.review.clone();
            pending.insert(review.intent_id, held);
            return Ok(Gate::Review(review));
        }
        if !proof.satisfies(held.review.proof, &held.review.subject) {
            return Err(GateError::WrongProof);
        }
        if let (ProofRequired::Passphrase, IntentProof::Passphrase(passphrase)) =
            (held.review.proof, proof)
        {
            let identity = owner.identity();
            if let Err(retry_after_ms) = self.limiter.check(identity, now_ms) {
                return Err(GateError::Locked { retry_after_ms });
            }
            if !passphrase_ok(passphrase) {
                self.limiter.fail(identity, now_ms);
                return Err(GateError::WrongProof);
            }
            self.limiter.succeed(identity);
        }
        pending
            .remove(&intent_id)
            .map(|p| Gate::Run(p.action))
            .ok_or(GateError::UnknownIntent)
    }

    /// Step 3 for hosts that answer with the run's result: commit, then
    /// run the action once through `run`. Concurrent commits of one id
    /// share that run, and a commit repeated within [`INTENT_TTL_MS`] gets
    /// its result without running again. A refusal or a fresh review is
    /// not remembered, so the user can correct the proof and commit again.
    ///
    /// # Errors
    /// As [`Self::commit`]; a repeated commit by another owner is
    /// [`GateError::UnknownIntent`].
    pub async fn commit_and_run<F, Fut>(
        &self,
        intent_id: IntentId,
        owner: &O,
        proof: &IntentProof,
        fresh: Selection,
        passphrase_ok: impl FnOnce(&str) -> bool + Send,
        now_ms: u64,
        run: F,
    ) -> Result<Committed<R>, GateError>
    where
        F: FnOnce(A) -> Fut + Send,
        Fut: Future<Output = R> + Send,
        A: Send,
    {
        let outcome = self
            .results
            .try_wrap(intent_id.0, || async {
                match self.commit(intent_id, owner, proof, fresh, passphrase_ok, now_ms) {
                    Ok(Gate::Run(action)) => Ok((owner.clone(), run(action).await)),
                    Ok(Gate::Review(review)) => Err(NotRun::Review(review)),
                    Err(e) => Err(NotRun::Refused(e)),
                }
            })
            .await;
        match outcome {
            Ok((ran_for, result)) if ran_for == *owner => Ok(Committed::Ran(result)),
            Ok(_) => Err(GateError::UnknownIntent),
            Err(not_run) => match &*not_run {
                NotRun::Review(review) => Ok(Committed::Review(review.clone())),
                NotRun::Refused(e) => Err(e.clone()),
            },
        }
    }

    /// The held action's selection, for recomputing its facts at commit.
    pub fn selection(&self, intent_id: IntentId, owner: &O) -> Option<Selection> {
        self.pending
            .lock()
            .get(&intent_id)
            .filter(|p| p.owner == *owner)
            .map(|p| p.selection.clone())
    }

    /// Drop a held action. Returns whether it was held for `owner`.
    pub fn cancel(&self, intent_id: IntentId, owner: &O) -> bool {
        let mut pending = self.pending.lock();
        if pending.get(&intent_id).is_some_and(|p| p.owner == *owner) {
            pending.remove(&intent_id);
            return true;
        }
        false
    }

    /// Drop every action `owner` holds (lock, logout, identity switch).
    pub fn clear(&self, owner: &O) {
        self.pending.lock().retain(|_, p| p.owner != *owner);
    }
}

#[cfg(test)]
mod tests;
