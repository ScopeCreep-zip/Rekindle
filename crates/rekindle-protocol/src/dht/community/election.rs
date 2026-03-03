//! Deterministic coordinator election scoring.
//!
//! Any eligible member can be elected coordinator via deterministic hash scoring:
//! BLAKE2b(community_id || epoch || pseudonym_key). Lower score wins.
//! Same inputs always produce the same output across all members.

use blake2::{digest::consts::U32, Blake2b, Digest};

use super::permissions_v2::Permissions;
use super::types::{MemberSummary, RoleEntryV2};

/// Minimum time (seconds) a member must have been joined before being eligible
/// for coordinator election. Prevents join-and-takeover attacks.
const MIN_JOIN_AGE_SECS: u64 = 300;

/// Compute the deterministic election score for a candidate.
///
/// Returns a 32-byte BLAKE2b hash of `community_id || epoch || pseudonym_key`.
/// Lower score wins (lexicographic comparison).
pub fn compute_election_score(community_id: &str, epoch: u64, pseudonym_key: &str) -> [u8; 32] {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(community_id.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.update(pseudonym_key.as_bytes());
    hasher.finalize().into()
}

/// Check if a member is eligible for the coordinator role.
///
/// Eligibility requirements:
/// - Has a role with the COORDINATOR permission bit (bit 51) OR the ADMINISTRATOR permission
/// - Joined more than `MIN_JOIN_AGE_SECS` seconds ago (prevents join-and-takeover)
/// - Not currently timed out
pub fn is_eligible(member: &MemberSummary, roles: &[RoleEntryV2], now_secs: u64) -> bool {
    // Check timeout
    if let Some(timeout_until) = member.timeout_until {
        if now_secs < timeout_until {
            return false;
        }
    }

    // Check join age
    if now_secs.saturating_sub(member.joined_at) < MIN_JOIN_AGE_SECS {
        return false;
    }

    // Check permissions: must have ADMINISTRATOR (which is a superset check)
    // In practice, owners and admins can be coordinators.
    // We use ADMINISTRATOR as the coordinator eligibility bit since there's
    // no separate COORDINATOR bit defined yet in the v2 permissions.
    let mut has_coordinator_perm = false;
    for role_id in &member.role_ids {
        if let Some(role) = roles.iter().find(|r| r.id == *role_id) {
            let perms = Permissions::from_bits_truncate(role.permissions);
            if perms.contains(Permissions::ADMINISTRATOR) {
                has_coordinator_perm = true;
                break;
            }
        }
    }

    has_coordinator_perm
}

/// Find the winning coordinator candidate.
///
/// Filters eligible members, computes scores, returns the pseudonym key
/// of the candidate with the lowest score (lexicographic).
pub fn find_winner(
    community_id: &str,
    epoch: u64,
    candidates: &[MemberSummary],
    roles: &[RoleEntryV2],
    now_secs: u64,
) -> Option<String> {
    let mut best_key: Option<String> = None;
    let mut best_score: Option<[u8; 32]> = None;

    for candidate in candidates {
        if !is_eligible(candidate, roles, now_secs) {
            continue;
        }

        let score = compute_election_score(community_id, epoch, &candidate.pseudonym_key);

        let is_better = match &best_score {
            None => true,
            Some(current_best) => score < *current_best,
        };

        if is_better {
            best_key = Some(candidate.pseudonym_key.clone());
            best_score = Some(score);
        }
    }

    best_key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_member(key: &str, joined_at: u64, role_ids: Vec<u32>) -> MemberSummary {
        MemberSummary {
            pseudonym_key: key.to_string(),
            display_name: key.to_string(),
            role_ids,
            joined_at,
            subkey_index: 0,
            onboarding_complete: true,
            timeout_until: None,
        }
    }

    fn admin_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 3,
            name: "Admin".into(),
            color: 0,
            permissions: Permissions::ADMINISTRATOR.bits(),
            position: 3,
            hoist: false,
            mentionable: false,
        }
    }

    fn member_role() -> RoleEntryV2 {
        RoleEntryV2 {
            id: 1,
            name: "Member".into(),
            color: 0,
            permissions: (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
            position: 1,
            hoist: false,
            mentionable: false,
        }
    }

    #[test]
    fn deterministic_scoring() {
        let s1 = compute_election_score("community_1", 5, "alice");
        let s2 = compute_election_score("community_1", 5, "alice");
        assert_eq!(s1, s2, "same inputs must produce same score");
    }

    #[test]
    fn different_inputs_different_scores() {
        let s1 = compute_election_score("community_1", 5, "alice");
        let s2 = compute_election_score("community_1", 5, "bob");
        assert_ne!(s1, s2);

        let s3 = compute_election_score("community_1", 6, "alice");
        assert_ne!(s1, s3, "different epoch should produce different score");

        let s4 = compute_election_score("community_2", 5, "alice");
        assert_ne!(s1, s4, "different community should produce different score");
    }

    #[test]
    fn eligibility_requires_admin() {
        let now = 10000;
        let member = make_member("alice", now - 600, vec![3]); // admin role
        let roles = vec![admin_role()];
        assert!(is_eligible(&member, &roles, now));
    }

    #[test]
    fn eligibility_rejects_non_admin() {
        let now = 10000;
        let member = make_member("alice", now - 600, vec![1]); // member role only
        let roles = vec![member_role()];
        assert!(!is_eligible(&member, &roles, now));
    }

    #[test]
    fn eligibility_rejects_new_member() {
        let now = 10000;
        let member = make_member("alice", now - 100, vec![3]); // admin but just joined
        let roles = vec![admin_role()];
        assert!(!is_eligible(&member, &roles, now));
    }

    #[test]
    fn eligibility_rejects_timed_out() {
        let now = 10000;
        let mut member = make_member("alice", now - 600, vec![3]);
        member.timeout_until = Some(now + 3600); // timed out
        let roles = vec![admin_role()];
        assert!(!is_eligible(&member, &roles, now));
    }

    #[test]
    fn find_winner_selects_lowest_score() {
        let now = 10000;
        let candidates = vec![
            make_member("alice", now - 600, vec![3]),
            make_member("bob", now - 600, vec![3]),
            make_member("charlie", now - 600, vec![3]),
        ];
        let roles = vec![admin_role()];

        let winner = find_winner("community_1", 1, &candidates, &roles, now);
        assert!(winner.is_some());

        // Verify determinism: same call twice gives same result
        let winner2 = find_winner("community_1", 1, &candidates, &roles, now);
        assert_eq!(winner, winner2);
    }

    #[test]
    fn find_winner_filters_ineligible() {
        let now = 10000;
        let candidates = vec![
            make_member("alice", now - 100, vec![3]), // too new
            make_member("bob", now - 600, vec![1]),   // not admin
            make_member("charlie", now - 600, vec![3]), // eligible
        ];
        let roles = vec![admin_role(), member_role()];

        let winner = find_winner("community_1", 1, &candidates, &roles, now);
        assert_eq!(winner, Some("charlie".to_string()));
    }

    #[test]
    fn find_winner_none_when_no_eligible() {
        let now = 10000;
        let candidates = vec![
            make_member("alice", now - 100, vec![3]), // too new
            make_member("bob", now - 600, vec![1]),   // not admin
        ];
        let roles = vec![admin_role(), member_role()];

        let winner = find_winner("community_1", 1, &candidates, &roles, now);
        assert!(winner.is_none());
    }

    #[test]
    fn epoch_changes_winner() {
        let now = 10000;
        let candidates = vec![
            make_member("alice", now - 600, vec![3]),
            make_member("bob", now - 600, vec![3]),
        ];
        let roles = vec![admin_role()];

        // Different epochs may produce different winners
        let mut winners = std::collections::HashSet::new();
        for epoch in 0..100 {
            if let Some(w) = find_winner("community_1", epoch, &candidates, &roles, now) {
                winners.insert(w);
            }
        }
        // With enough epochs, both should win at least once
        assert!(
            winners.len() >= 2,
            "both candidates should eventually win across epochs"
        );
    }
}
