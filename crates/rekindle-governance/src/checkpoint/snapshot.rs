//! A decoded state snapshot (plan E3.2 S7c.4a): the state and the cut index
//! at a checkpoint's cut, which `merge::merge_from` resumes from, with the
//! cut's heads from the checkpoint body.

use rekindle_types::governance::{Checkpoint, OpHash};

use super::cut::{state_preimage, state_root_of, CutIndex};
use crate::checksum::decode::{Decanon, Reader};
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
        let mut r = Reader::new(bytes);
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
