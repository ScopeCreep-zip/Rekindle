//! CRDT merge engine — the core of flat governance.
//!
//! Takes all `GovernanceEntry` variants from all member subkeys,
//! processes them in deterministic order, and produces a `GovernanceState`.
//!
//! **Convergence guarantee:** Given the same set of entries (in any order),
//! `merge()` always produces an identical `GovernanceState`. This is verified
//! by property-based tests.
//!
//! **Genesis bypass:** The first entry (lowest lamport) is always accepted
//! regardless of permissions — this bootstraps the community before any
//! role structure exists.
//!
//! **Reader-validates:** After genesis, each entry is checked against the
//! accumulated permission state. Entries from members without the required
//! permission are silently excluded.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::state::*;
use crate::validate::validate_write;

/// A single entry tagged with its author's pseudonym.
#[derive(Debug, Clone)]
pub struct AuthoredEntry {
    pub author: PseudonymKey,
    pub entry: GovernanceEntry,
}

/// Merge governance entries from all member subkeys into a canonical state.
///
/// # Arguments
/// * `subkeys` — One vec of entries per member subkey. Each tuple is
///   `(author_pseudonym, entries_from_that_subkey)`.
///
/// # Returns
/// A deterministic `GovernanceState` that all peers agree on.
pub fn merge(subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)]) -> GovernanceState {
    // 1. Collect all entries with their author
    let mut all: Vec<AuthoredEntry> = Vec::new();
    for (author, entries) in subkeys {
        for entry in entries {
            all.push(AuthoredEntry {
                author: author.clone(),
                entry: entry.clone(),
            });
        }
    }

    // 2. Sort by (lamport, author_pseudonym) for deterministic total order
    all.sort_by(|a, b| {
        a.entry
            .lamport()
            .cmp(&b.entry.lamport())
            .then_with(|| a.author.0.cmp(&b.author.0))
    });

    // 3. Process in order, applying CRDT rules
    let mut state = GovernanceState::default();

    for (idx, authored) in all.iter().enumerate() {
        let is_genesis = idx == 0;

        if is_genesis {
            // Genesis entry always accepted — bootstraps the community
            state.creator = Some(authored.author.clone());
            apply(&authored.author, &authored.entry, &mut state);
        } else if validate_write(&authored.author, &authored.entry, &state) {
            apply(&authored.author, &authored.entry, &mut state);
        }
        // else: silently excluded (reader-validates)
    }

    state
}

/// Apply a single governance entry to the accumulated state.
///
/// Each entry type has its own CRDT merge rule (see architecture doc §4.4).
/// Public as `apply_entry` for incremental local updates (after permission validation).
pub fn apply_entry(author: &PseudonymKey, entry: &GovernanceEntry, state: &mut GovernanceState) {
    apply(author, entry, state);
}

/// Route an entry to its category-specific CRDT apply rule.
fn apply(author: &PseudonymKey, entry: &GovernanceEntry, state: &mut GovernanceState) {
    use GovernanceEntry as G;
    match entry {
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::CategoryCreated { .. }
        | G::CategoryArchived { .. }
        | G::CategoryUpdated { .. }
        | G::PermissionOverwrite { .. }
        | G::ChannelSegmentLinked { .. } => apply_channels(entry, state),
        G::JoinRequested { .. } | G::MemberApproved { .. } | G::MemberRejected { .. } => {
            apply_admission(entry, state);
        }
        // `validate_write` already refused any write after the first,
        // so reaching here means this is the creator's one-and-only
        // policy entry.
        G::AdmissionPolicy { mode, .. } => state.admission_mode = Some(*mode),
        G::CommunityMeta { .. }
        | G::CommunityNotificationDefault { .. }
        | G::MEKGenerationBump { .. }
        | G::SegmentAdded { .. }
        | G::CommunityPolicy { .. }
        | G::InviteCreated { .. }
        | G::InviteRevoked { .. } => apply_community(author, entry, state),
        G::ThreadCreated { .. }
        | G::ThreadArchived { .. }
        | G::EventCreated { .. }
        | G::EventArchived { .. } => apply_events(author, entry, state),
        G::ExpressionAdded { .. } | G::ExpressionRemoved { .. } | G::AttachmentPinned { .. } => {
            apply_expression(entry, state)
        }
        G::BanEntry { .. }
        | G::UnbanEntry { .. }
        | G::TimeoutEntry { .. }
        | G::RemoveTimeoutEntry { .. }
        | G::AdminDelete { .. }
        | G::AutoModRule { .. } => apply_moderation(entry, state),
        G::OnboardingConfig { .. } | G::WelcomeScreen { .. } => apply_onboarding(entry, state),
        G::RoleDefinition { .. }
        | G::RoleAssignment { .. }
        | G::RoleUnassignment { .. }
        | G::RoleArchived { .. } => apply_roles(entry, state),
    }
}

mod channels;
mod community;
mod events;
mod expression;
mod moderation;
mod onboarding;
mod roles;

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;

use channels::apply_channels;
use community::apply_community;
use events::apply_events;
use expression::apply_expression;
use moderation::{apply_admission, apply_moderation};
use onboarding::apply_onboarding;
use roles::apply_roles;
