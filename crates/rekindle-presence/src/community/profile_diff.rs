//! Pure member-profile diff for the registry-scan post-processing.
//!
//! Architecture wave 5 / D1 — emit `CommunityEvent::MembersRefreshed`
//! only when at least one profile field actually changed. Before this
//! every 30s presence tick fired the event regardless, which the
//! frontend treats as a re-fetch trigger — wasted IPC bandwidth.
//!
//! [`compute_profile_diff`] takes the prior cached map + the
//! freshly-scanned rows and returns:
//! - `updates`: the `(pseudonym_hex, MemberProfileSnapshot)` pairs the
//!   adapter writes back into `community.member_profiles`.
//! - `changed`: whether at least one entry differed from the prior
//!   snapshot — drives the `MembersRefreshed` emit gate.

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::community::DiscoveredRow;

/// Mirror of src-tauri's `MemberProfileSnapshot` — the per-member
/// profile fields the registry presence row publishes. The adapter
/// converts between this DTO and the AppState type with a thin map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberProfileSnapshot {
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
}

/// Outcome of the diff: the rows to write back + whether the
/// adapter should fire `MembersRefreshed`.
#[derive(Debug, Default)]
pub struct ProfileDiffOutcome {
    pub updates: HashMap<String, MemberProfileSnapshot>,
    pub changed: bool,
}

/// Compute the profile-diff outcome for one presence-poll tick.
///
/// `prior_snapshots` is the existing `community.member_profiles`
/// map (read once under a snapshot lock; the adapter writes the
/// merged result back atomically). `discovered` is the freshly
/// scanned + verified presence rows.
///
/// `updates` contains ONLY the entries that differ — the adapter
/// extends the in-memory map with these. `changed` is `true` iff
/// `updates` is non-empty, gating the `MembersRefreshed` emit.
#[must_use]
pub fn compute_profile_diff<S: BuildHasher>(
    prior_snapshots: &HashMap<String, MemberProfileSnapshot, S>,
    discovered: &[DiscoveredRow],
) -> ProfileDiffOutcome {
    let mut updates: HashMap<String, MemberProfileSnapshot> = HashMap::new();
    for (_segment_index, _subkey, presence) in discovered {
        let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
        let next = MemberProfileSnapshot {
            display_name: presence.display_name.clone(),
            bio: presence.bio.clone(),
            pronouns: presence.pronouns.clone(),
            theme_color: presence.theme_color,
            badges: presence.badges.clone(),
            avatar_ref: presence.avatar_ref.clone(),
            banner_ref: presence.banner_ref.clone(),
        };
        let prev = prior_snapshots.get(&pseudonym_hex);
        if prev.is_none_or(|existing| existing != &next) {
            updates.insert(pseudonym_hex, next);
        }
    }
    let changed = !updates.is_empty();
    ProfileDiffOutcome { updates, changed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::PseudonymKey;
    use rekindle_types::presence::MemberPresence;

    fn row(pk: u8, display: Option<&str>, bio: Option<&str>) -> DiscoveredRow {
        let mut bytes = [0u8; 32];
        bytes[0] = pk;
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey(bytes),
            display_name: display.map(str::to_string),
            bio: bio.map(str::to_string),
            ..Default::default()
        };
        (0u32, 0u32, presence)
    }

    fn snapshot(display: Option<&str>, bio: Option<&str>) -> MemberProfileSnapshot {
        MemberProfileSnapshot {
            display_name: display.map(str::to_string),
            bio: bio.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn empty_discovered_yields_no_changes() {
        let prior: HashMap<String, MemberProfileSnapshot> = HashMap::new();
        let outcome = compute_profile_diff(&prior, &[]);
        assert!(!outcome.changed);
        assert!(outcome.updates.is_empty());
    }

    #[test]
    fn unchanged_row_is_not_flagged() {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        let pk_hex = hex::encode(bytes);
        let mut prior = HashMap::new();
        prior.insert(pk_hex.clone(), snapshot(Some("alice"), Some("hi")));
        let outcome = compute_profile_diff(&prior, &[row(1, Some("alice"), Some("hi"))]);
        assert!(!outcome.changed);
        assert!(outcome.updates.is_empty());
    }

    #[test]
    fn changed_display_name_flagged() {
        let mut bytes = [0u8; 32];
        bytes[0] = 2;
        let pk_hex = hex::encode(bytes);
        let mut prior = HashMap::new();
        prior.insert(pk_hex.clone(), snapshot(Some("alice"), Some("hi")));
        let outcome = compute_profile_diff(&prior, &[row(2, Some("alice-renamed"), Some("hi"))]);
        assert!(outcome.changed);
        assert_eq!(outcome.updates.len(), 1);
        assert_eq!(
            outcome.updates.get(&pk_hex).unwrap().display_name,
            Some("alice-renamed".to_string())
        );
    }

    #[test]
    fn new_row_is_flagged_and_added_to_updates() {
        let prior: HashMap<String, MemberProfileSnapshot> = HashMap::new();
        let outcome = compute_profile_diff(&prior, &[row(3, Some("bob"), None)]);
        assert!(outcome.changed);
        assert_eq!(outcome.updates.len(), 1);
    }

    #[test]
    fn missing_field_added_in_new_snapshot_is_flagged() {
        let mut bytes = [0u8; 32];
        bytes[0] = 4;
        let pk_hex = hex::encode(bytes);
        let mut prior = HashMap::new();
        prior.insert(pk_hex.clone(), snapshot(Some("alice"), None));
        let outcome = compute_profile_diff(&prior, &[row(4, Some("alice"), Some("now has bio"))]);
        assert!(outcome.changed);
    }

    #[test]
    fn multiple_rows_only_changed_one_yields_single_update() {
        let mut bytes1 = [0u8; 32];
        bytes1[0] = 5;
        let pk1 = hex::encode(bytes1);
        let mut bytes2 = [0u8; 32];
        bytes2[0] = 6;
        let pk2 = hex::encode(bytes2);
        let mut prior = HashMap::new();
        prior.insert(pk1.clone(), snapshot(Some("alice"), None));
        prior.insert(pk2.clone(), snapshot(Some("bob"), None));
        let outcome = compute_profile_diff(
            &prior,
            &[
                row(5, Some("alice"), None),       // unchanged
                row(6, Some("bob-renamed"), None), // changed
            ],
        );
        assert!(outcome.changed);
        assert_eq!(outcome.updates.len(), 1);
        assert!(outcome.updates.contains_key(&pk2));
    }
}
