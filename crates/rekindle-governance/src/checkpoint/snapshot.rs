//! A decoded state snapshot (plan E3.2 S7c.4a): the state and the cut index
//! at a checkpoint's cut, which `merge::merge_from` resumes from, with the
//! cut's heads from the checkpoint body.

use rekindle_types::governance::{Checkpoint, GenesisCert, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::cut::{state_preimage, state_root_of, CutIndex, CutOp};
use crate::checksum::decode::{CanonReader, Decanon};
use crate::state::GovernanceState;

/// The state and cut index a checkpoint's `stateRoot` commits to.
#[derive(Debug, Clone, PartialEq)]
pub struct CutSnapshot {
    pub state: GovernanceState,
    pub cut: CutIndex,
    /// The cut's heads (the checkpoint's `cut`): the cut is their causal
    /// closure, and no cut op has a head in its causal past.
    pub heads: Vec<OpHash>,
}

impl CutSnapshot {
    /// Decode `bytes`, a snapshot served for `checkpoint`: `None` unless
    /// they hash to its `stateRoot`, parse whole, and re-encode to the same
    /// bytes (so only the canonical encoding is ever kept).
    #[must_use]
    pub fn decode(checkpoint: &Checkpoint, bytes: &[u8]) -> Option<Self> {
        if state_root_of(bytes) != checkpoint.state_root {
            return None;
        }
        let mut r = CanonReader::new(bytes);
        let state = GovernanceState::take(&mut r)?;
        let cut = CutIndex {
            authors: Decanon::take(&mut r)?,
            power: Decanon::take(&mut r)?,
            power_heads: Decanon::take(&mut r)?,
            mainline_max: Decanon::take(&mut r)?,
            holders: Decanon::take(&mut r)?,
            masked: Decanon::take(&mut r)?,
            invite_codes: Decanon::take(&mut r)?,
            forks: Decanon::take(&mut r)?,
            unmerged: Decanon::take(&mut r)?,
            lamports: Decanon::take(&mut r)?,
            checkpoints: Decanon::take(&mut r)?,
        };
        if !r.is_done() || state_preimage(&state, &cut) != bytes {
            return None;
        }
        Some(Self {
            state,
            cut,
            heads: checkpoint.cut.iter().map(|r| r.op_hash).collect(),
        })
    }
}

impl CutSnapshot {
    /// Where the cut op `op` lives: its author's member record in the
    /// snapshot's state (the directory at the cut, member → record; the
    /// cert's for the creator) at its seq, the locator a reader fetches it
    /// by (plan E3.2 S7c.5, P3). `None` when the state names no record for
    /// its author.
    #[must_use]
    pub fn locate(&self, cert: &GenesisCert, op: &CutOp) -> Option<OpRef> {
        let author = PseudonymKey(op.author);
        Some(OpRef {
            op_hash: op.hash,
            author_record: self.state.author_record(cert, &author)?,
            author,
            author_seq: op.seq,
        })
    }

    /// The locators of the cut's applied power ops, in the power phase's
    /// order: what a reader holding only the snapshot fetches for
    /// `merge_from` (a refused one is never folded, and its author, a
    /// non-member, may have no record at the cut). `None` when one cannot
    /// be located.
    #[must_use]
    pub fn power_refs(&self, cert: &GenesisCert) -> Option<Vec<OpRef>> {
        (self.cut.power.iter())
            .filter(|(_, applied)| *applied)
            .map(|(op, _)| self.locate(cert, op))
            .collect()
    }
}
