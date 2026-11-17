//! The governance checkpoint op body (plan E3.2 S7c.3): an acceleration
//! hint a joiner starts from, never authoritative. The op itself is its
//! author's endorsement, signed under `rekindle-gov-op-v1` like every op.

use serde::{Deserialize, Serialize};

use super::op::OpRef;

/// A `Checkpoint` op's body. Its digest (`rekindle_governance::checkpoint`,
/// BLAKE3 `derive_key("rekindle-gov-checkpoint-v1")`) covers every field
/// but `pages` and `round`: page locators differ by writer while the
/// content does not, and the round only numbers the vote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// 1 for the first checkpoint, then `prevCheckpoint`'s height + 1.
    pub height: u64,
    /// The endorsement round this op votes in (IC5).
    pub round: u32,
    /// The previous checkpoint op (`prevCheckpoint` on the wire); `None`
    /// only at height 1.
    pub prev: Option<OpRef>,
    /// The cut's heads: the cut is their causal closure. At most
    /// `MAX_FRONTIER_REFS - 1`, and every one is among the op's deps.
    pub cut: Vec<OpRef>,
    /// BLAKE3 over the cut's head hashes, sorted.
    pub frontier_hash: [u8; 32],
    /// The directory's RFC 9162 root at this checkpoint (S7c.2).
    pub dir_root: [u8; 32],
    /// The directory's leaf count.
    pub dir_size: u64,
    /// The merged state at the cut and the cut index, digested.
    pub state_root: [u8; 32],
    /// `state.mek_generation` at the cut.
    pub mek_generation: u64,
    /// The electorate: the sorted pseudonyms of the admitted, non-removed
    /// members at `prevCheckpoint`'s cut (the cert's creator set at
    /// height 1), digested.
    pub eligible_set_hash: [u8; 32],
    /// DHT keys of the directory pages this writer sealed, in page order.
    pub pages: Vec<String>,
}
