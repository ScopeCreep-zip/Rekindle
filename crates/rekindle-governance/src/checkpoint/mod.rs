//! Governance checkpoints (plan E3.2 S7c): acceleration hints a joiner
//! starts from, never authoritative.

mod cut;
pub mod directory;
mod electorate;
mod shape;

pub use cut::{canonical_cut, state_preimage, state_root, state_root_of, CutIndex};
pub use electorate::{eligible_set, eligible_set_hash, is_strict_majority, tally};
pub use shape::{check_shape, checkpoint_digest, frontier_hash, MAX_CUT_HEADS};
