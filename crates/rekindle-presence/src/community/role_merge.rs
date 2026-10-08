//! Pure role-merge logic for the registry-scan post-processing.
//!
//! Composes three role sources with priority order (lowest →
//! highest): existing `member_roles` ← discovered governance
//! assignments ← local user's `my_role_ids`. The adapter exposes
//! finer-grained primitives (`read_existing_member_roles`,
//! `read_governance_role_assignments`, `read_my_role_ids`) and the
//! crate orchestrator composes them via [`compute_merged_roles`]
//! before handing the result back to the host for application
//! (`apply_member_state_update`).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use rekindle_types::id::{PseudonymKey, RoleId};

use crate::community::util::role_ids_from_governance;
use crate::community::DiscoveredRow;

/// Compose the merged `member_roles` map for one presence-poll tick.
///
/// Priority order (highest wins):
/// 1. `my_role_ids` for `my_pseudonym` — local override; the user
///    knows their own roles authoritatively.
/// 2. Governance `role_assignments` for each discovered pseudonym
///    — peer's role state per the merged CRDT.
/// 3. `existing_member_roles` for any pseudonym not seen in this
///    scan — preserves prior knowledge.
///
/// `my_pseudonym` empty → step 1 skipped (defensive — we don't
/// know who "we" are yet).
#[must_use]
pub fn compute_merged_roles<S1, S2, S3>(
    existing_member_roles: HashMap<String, Vec<u32>, S1>,
    discovered: &[DiscoveredRow],
    governance_role_assignments: &HashMap<PseudonymKey, HashSet<RoleId, S2>, S3>,
    my_pseudonym: &str,
    my_role_ids: Vec<u32>,
) -> HashMap<String, Vec<u32>>
where
    S1: BuildHasher,
    S2: BuildHasher,
    S3: BuildHasher,
{
    // Seed with prior state so any peer absent from this scan still
    // has their last-known role set.
    let mut merged: HashMap<String, Vec<u32>> = existing_member_roles.into_iter().collect();

    // Overlay discovered peers' governance-mandated roles.
    for (_segment_index, _subkey, presence) in discovered {
        let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
        let role_ids = governance_role_assignments
            .get(&presence.pseudonym_key)
            .map_or_else(|| vec![0], role_ids_from_governance);
        merged.insert(pseudonym_hex, role_ids);
    }

    // Local override for our own pseudonym — we know our roles
    // authoritatively, governance state may lag.
    if !my_pseudonym.is_empty() {
        merged.insert(my_pseudonym.to_string(), my_role_ids);
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(first: u8) -> RoleId {
        let mut bytes = [0u8; 16];
        bytes[0] = first;
        RoleId(bytes)
    }

    fn pseudonym(first: u8) -> PseudonymKey {
        let mut bytes = [0u8; 32];
        bytes[0] = first;
        PseudonymKey(bytes)
    }

    fn discovered_for(pk: PseudonymKey) -> DiscoveredRow {
        use rekindle_types::presence::MemberPresence;
        let presence = MemberPresence {
            pseudonym_key: pk,
            ..Default::default()
        };
        (0u32, 0u32, presence)
    }

    #[test]
    fn empty_inputs_yield_empty_map() {
        let merged: HashMap<String, Vec<u32>> = compute_merged_roles(
            HashMap::<String, Vec<u32>>::new(),
            &[],
            &HashMap::<PseudonymKey, HashSet<RoleId>>::new(),
            "",
            Vec::new(),
        );
        assert!(merged.is_empty());
    }

    #[test]
    fn discovered_with_no_governance_assignment_gets_default_role() {
        let pk = pseudonym(1);
        let merged = compute_merged_roles(
            HashMap::<String, Vec<u32>>::new(),
            &[discovered_for(pk.clone())],
            &HashMap::<PseudonymKey, HashSet<RoleId>>::new(),
            "",
            Vec::new(),
        );
        let hex_pk = hex::encode(pk.0);
        assert_eq!(merged.get(&hex_pk), Some(&vec![0]));
    }

    #[test]
    fn discovered_inherits_governance_role_ids_sorted() {
        let pk = pseudonym(2);
        let mut assignments: HashMap<PseudonymKey, HashSet<RoleId>> = HashMap::new();
        let mut set = HashSet::new();
        set.insert(role(7));
        set.insert(role(2));
        assignments.insert(pk.clone(), set);
        let merged = compute_merged_roles(
            HashMap::<String, Vec<u32>>::new(),
            &[discovered_for(pk.clone())],
            &assignments,
            "",
            Vec::new(),
        );
        let hex_pk = hex::encode(pk.0);
        assert_eq!(merged.get(&hex_pk), Some(&vec![2, 7]));
    }

    #[test]
    fn my_pseudonym_override_wins_over_governance() {
        let my_pk_hex = "deadbeef".to_string();
        let mut existing: HashMap<String, Vec<u32>> = HashMap::new();
        existing.insert(my_pk_hex.clone(), vec![99]); // stale prior
        let merged = compute_merged_roles(
            existing,
            &[],
            &HashMap::<PseudonymKey, HashSet<RoleId>>::new(),
            &my_pk_hex,
            vec![1, 2, 3],
        );
        assert_eq!(merged.get(&my_pk_hex), Some(&vec![1, 2, 3]));
    }

    #[test]
    fn existing_preserved_for_peers_absent_from_scan() {
        let mut existing: HashMap<String, Vec<u32>> = HashMap::new();
        existing.insert("alice".to_string(), vec![5]);
        existing.insert("bob".to_string(), vec![6]);
        let merged = compute_merged_roles(
            existing,
            &[],
            &HashMap::<PseudonymKey, HashSet<RoleId>>::new(),
            "",
            Vec::new(),
        );
        assert_eq!(merged.get("alice"), Some(&vec![5]));
        assert_eq!(merged.get("bob"), Some(&vec![6]));
    }

    #[test]
    fn empty_my_pseudonym_skips_local_override() {
        let mut existing: HashMap<String, Vec<u32>> = HashMap::new();
        existing.insert(String::new(), vec![999]);
        let merged = compute_merged_roles(
            existing,
            &[],
            &HashMap::<PseudonymKey, HashSet<RoleId>>::new(),
            "", // empty — no override
            vec![1, 2, 3],
        );
        // The empty-string entry from `existing` is preserved
        // (not overwritten by `my_role_ids`).
        assert_eq!(merged.get(""), Some(&vec![999]));
    }
}
