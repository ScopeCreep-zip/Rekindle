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
    /// The member's focused channel, decoded from `session.location`.
    /// Included in the diff so a channel move flags `MembersRefreshed`
    /// (the roster re-fetches location via the same one-flow path); the
    /// live value the DTO renders still comes from the online overlay.
    pub location: Option<rekindle_types::presence::SessionLocation>,
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
            location: presence.session.location.clone(),
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
#[path = "profile_diff_tests.rs"]
mod tests;
