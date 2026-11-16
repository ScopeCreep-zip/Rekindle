//! D15's convergence check (plan E3.2, Tox `peers_checksum`): two members
//! holding the same op set merge byte-identical roster and role tables, so
//! a hash of them, published in the `Frontier` beside the heads, tells a
//! reader with the same heads whether its merge agrees.
//!
//! The canonical encoding: three sections — members, roles, role
//! assignments — each a big-endian `u32` count and its rows sorted by key
//! bytes (the `HashMap`s and `HashSet`s are never iterated in their own
//! order); strings are a `u32` length and their UTF-8 bytes; an optional
//! value is a `0` byte, or a `1` byte and the value.

use rekindle_types::domains;
use rekindle_types::governance::RosterChecksum;

use crate::op::OpStamp;
use crate::state::GovernanceState;

/// The roster checksum of `state`: BLAKE3 `derive_key(GOV_ROSTER_CHECKSUM)`
/// over the canonical encoding of its members, roles and role assignments.
#[must_use]
pub fn roster_checksum(state: &GovernanceState) -> RosterChecksum {
    let mut hasher = blake3::Hasher::new_derive_key(domains::GOV_ROSTER_CHECKSUM);
    hasher.update(&canonical_roster(state));
    RosterChecksum(*hasher.finalize().as_bytes())
}

/// The canonical encoding [`roster_checksum`] hashes.
fn canonical_roster(state: &GovernanceState) -> Vec<u8> {
    let mut out = Vec::new();

    let mut members: Vec<_> = state.members.iter().collect();
    members.sort_by_key(|(key, _)| key.0);
    count(&mut out, members.len());
    for (key, member) in members {
        out.extend_from_slice(&key.0);
        text(&mut out, &member.member_record);
        text(&mut out, &member.member_record_owner);
        stamp(&mut out, member.joined);
    }

    let mut roles: Vec<_> = state.roles.iter().collect();
    roles.sort_by_key(|(id, _)| id.0);
    count(&mut out, roles.len());
    for (id, role) in roles {
        out.extend_from_slice(&id.0);
        text(&mut out, &role.name);
        out.extend_from_slice(&role.permissions.to_be_bytes());
        out.extend_from_slice(&role.position.to_be_bytes());
        out.extend_from_slice(&role.color.to_be_bytes());
        out.extend_from_slice(&[
            u8::from(role.hoist),
            u8::from(role.mentionable),
            u8::from(role.self_assignable),
        ]);
        match &role.exclusion_group {
            None => out.push(0),
            Some(group) => {
                out.push(1);
                text(&mut out, group);
            }
        }
        stamp(&mut out, role.stamp);
    }

    let mut assignments: Vec<_> = state.role_assignments.iter().collect();
    assignments.sort_by_key(|(key, _)| key.0);
    count(&mut out, assignments.len());
    for (key, held) in assignments {
        out.extend_from_slice(&key.0);
        let mut held: Vec<[u8; 16]> = held.iter().map(|id| id.0).collect();
        held.sort_unstable();
        count(&mut out, held.len());
        for id in held {
            out.extend_from_slice(&id);
        }
    }
    out
}

fn count(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes());
}

fn text(out: &mut Vec<u8>, s: &str) {
    count(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

fn stamp(out: &mut Vec<u8>, stamp: OpStamp) {
    out.extend_from_slice(&stamp.lamport.to_be_bytes());
    out.extend_from_slice(&stamp.hash.0);
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
