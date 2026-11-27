//! D15's convergence check (plan E3.2, Tox `peers_checksum`): two members
//! holding the same op set merge byte-identical state, so a hash of it,
//! published in the `Frontier` beside the heads, tells a reader with the
//! same heads whether its merge agrees.
//!
//! One canonical encoding, `canonical_state` (plan E3.2 S7c.3): every field
//! of the merged state, each a section of a big-endian `u32` count and its
//! rows sorted by key bytes ([`canon`]). The `Frontier`'s roster checksum
//! hashes it alone; a checkpoint's `stateRoot` hashes it followed by the
//! cut index (`crate::checkpoint::state_root`), so the two cannot disagree
//! about the state and D15 covers every state field.

use rekindle_types::domains;
use rekindle_types::governance::RosterChecksum;

use crate::state::GovernanceState;

pub(crate) mod canon;
pub(crate) mod decode;
mod devices;
mod state;

pub use state::canonical_state;

/// The roster checksum of `state`: BLAKE3 `derive_key(GOV_ROSTER_CHECKSUM)`
/// over [`canonical_state`].
#[must_use]
pub fn roster_checksum(state: &GovernanceState) -> RosterChecksum {
    let mut hasher = blake3::Hasher::new_derive_key(domains::GOV_ROSTER_CHECKSUM);
    hasher.update(&canonical_state(state));
    RosterChecksum(*hasher.finalize().as_bytes())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
mod state_tests;
