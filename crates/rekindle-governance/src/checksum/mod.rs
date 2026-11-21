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
mod tests {
    use std::collections::HashSet;

    use rekindle_types::id::{PseudonymKey, RoleId};

    use super::roster_checksum;
    use crate::op::OpStamp;
    use crate::state::{GovernanceState, MemberState, RoleState};

    fn member(n: u8) -> (PseudonymKey, MemberState) {
        (
            PseudonymKey([n; 32]),
            MemberState {
                member_record: format!("VLD0:m{n}"),
                member_record_owner: format!("o{n}"),
                joined: OpStamp::MIN,
            },
        )
    }

    fn role(n: u8) -> (RoleId, RoleState) {
        (
            RoleId([n; 16]),
            RoleState {
                name: format!("r{n}"),
                permissions: u64::from(n),
                position: u32::from(n),
                color: 0,
                hoist: false,
                mentionable: n.is_multiple_of(2),
                self_assignable: false,
                exclusion_group: None,
                stamp: OpStamp::MIN,
            },
        )
    }

    /// `state` built by inserting rows in `order`.
    fn state(order: &[u8]) -> GovernanceState {
        let mut state = GovernanceState::default();
        for &n in order {
            let (key, m) = member(n);
            state.members.insert(key.clone(), m);
            let (id, r) = role(n);
            state.roles.insert(id, r);
            state
                .role_assignments
                .entry(key)
                .or_insert_with(HashSet::new)
                .extend(order.iter().map(|&k| RoleId([k; 16])));
        }
        state
    }

    /// The same tables inserted in a different order hash the same; a
    /// changed row does not.
    #[test]
    fn the_checksum_does_not_depend_on_insertion_order() {
        let forward: Vec<u8> = (1..=40).collect();
        let backward: Vec<u8> = forward.iter().rev().copied().collect();
        let a = state(&forward);
        let b = state(&backward);
        assert_eq!(roster_checksum(&a), roster_checksum(&b));
        let mut c = b.clone();
        if let Some(r) = c.roles.get_mut(&RoleId([7; 16])) {
            r.permissions ^= 1;
        }
        assert_ne!(roster_checksum(&a), roster_checksum(&c));
        let mut d = b;
        d.members.remove(&PseudonymKey([3; 32]));
        assert_ne!(roster_checksum(&a), roster_checksum(&d));
    }
}

#[cfg(test)]
mod state_tests;
