//! Governance checkpoints (plan E3.2 S7c): acceleration hints a joiner
//! starts from, never authoritative.

mod body;
mod chain;
mod cut;
pub mod directory;
mod electorate;
mod endorse;
mod hints;
mod shape;
mod snapshot;

pub use body::Replay;
pub use chain::{endorsed_link, pages_match, EndorsedLink, SignedEndorsement};
pub use cut::{
    canonical_cut, causal_past, state_preimage, state_root, state_root_of, CutIndex, CutOp,
};
pub use electorate::{eligible_set, eligible_set_hash, is_strict_majority, tally};
pub use endorse::{beyond, endorsement, proposal_due, proposals, Beyond, PROPOSAL_OPS};
pub use hints::{hints, EndorsementFault, Equivocation, Hints, UsableHint};
pub use shape::{check_shape, checkpoint_digest, frontier_hash, MAX_CUT_HEADS};
pub use snapshot::CutSnapshot;
