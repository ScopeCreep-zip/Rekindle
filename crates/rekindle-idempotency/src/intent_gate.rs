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
//! - A held action belongs to the owner that began it (the host's
//!   connection and identity), expires after [`INTENT_TTL_MS`], and at
//!   most [`MAX_PENDING_PER_OWNER`] are held per owner. Lock, logout and
//!   identity switch [`IntentGate::clear`] the owner's.

use std::collections::HashMap;

use rekindle_types::intent::{
    effective_class, Escalation, Fact, IntentClass, IntentKind, IntentProof, IntentReview,
    Notifies, ProofRequired, Reversibility, Subject,
};
use uuid::Uuid;

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

/// Why a commit or selection was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    #[error("no held action with this id (expired, cancelled, already run or not ours)")]
    UnknownIntent,
    #[error("the proof does not match what the review asked for")]
    WrongProof,
    #[error("too many actions are waiting for review; finish or cancel one")]
    TooManyPending,
}

struct Pending<A, O> {
    owner: O,
    selection: Selection,
    review: IntentReview,
    action: A,
}

/// Held actions of type `A`, owned by `O`.
pub struct IntentGate<A, O> {
    pending: parking_lot::Mutex<HashMap<String, Pending<A, O>>>,
}

impl<A, O> Default for IntentGate<A, O> {
    fn default() -> Self {
        Self {
            pending: parking_lot::Mutex::new(HashMap::new()),
        }
    }
}

fn review_of(selection: &Selection, now_ms: u64) -> IntentReview {
    let class = effective_class(selection.kind, &selection.escalations);
    IntentReview {
        intent_id: Uuid::now_v7().to_string(),
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

impl<A, O: PartialEq + Clone> IntentGate<A, O> {
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
            review.intent_id.clone(),
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
        intent_id: &str,
        owner: &O,
        proof: &IntentProof,
        fresh: Selection,
        passphrase_ok: impl FnOnce(&str) -> bool,
        now_ms: u64,
    ) -> Result<Gate<A>, GateError> {
        let mut pending = self.pending.lock();
        let held = pending
            .get(intent_id)
            .filter(|p| p.owner == *owner && p.review.expires_at_ms > now_ms)
            .ok_or(GateError::UnknownIntent)?;
        if fresh != held.selection {
            let Some(mut held) = pending.remove(intent_id) else {
                return Err(GateError::UnknownIntent);
            };
            held.review = review_of(&fresh, now_ms);
            held.selection = fresh;
            let review = held.review.clone();
            pending.insert(review.intent_id.clone(), held);
            return Ok(Gate::Review(review));
        }
        if !proof.satisfies(held.review.proof, &held.review.subject) {
            return Err(GateError::WrongProof);
        }
        if let (ProofRequired::Passphrase, IntentProof::Passphrase(passphrase)) =
            (held.review.proof, proof)
        {
            if !passphrase_ok(passphrase) {
                return Err(GateError::WrongProof);
            }
        }
        pending
            .remove(intent_id)
            .map(|p| Gate::Run(p.action))
            .ok_or(GateError::UnknownIntent)
    }

    /// The held action's selection, for recomputing its facts at commit.
    pub fn selection(&self, intent_id: &str, owner: &O) -> Option<Selection> {
        self.pending
            .lock()
            .get(intent_id)
            .filter(|p| p.owner == *owner)
            .map(|p| p.selection.clone())
    }

    /// Drop a held action. Returns whether it was held for `owner`.
    pub fn cancel(&self, intent_id: &str, owner: &O) -> bool {
        let mut pending = self.pending.lock();
        if pending.get(intent_id).is_some_and(|p| p.owner == *owner) {
            pending.remove(intent_id);
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
