//! Reader-validates: check if a writer had permission for a governance entry.
//!
//! Every peer independently validates incoming governance entries against the
//! CRDT-merged permission state. Invalid entries are silently excluded from
//! the materialized view.
//!
//! See architecture doc §9.3 for the enforcement model.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{
    ADMINISTRATOR, BAN_MEMBERS, CREATE_EVENTS, CREATE_EXPRESSIONS, CREATE_INVITES, MANAGE_CHANNELS,
    MANAGE_COMMUNITY, MANAGE_EVENTS, MANAGE_EXPRESSIONS, MANAGE_MESSAGES, MANAGE_ROLES,
    MANAGE_THREADS, SEND_MESSAGES, TIMEOUT_MEMBERS,
};

use crate::permissions::compute_permissions;
use crate::state::GovernanceState;

mod automod;
mod limits;
mod threads;

use automod::validate_automod_rule;
use limits::{expression_within_limits, onboarding_within_limits, soundboard_meta_valid};
use threads::validate_thread_create;

// Architecture §19.3 — welcome screen featured channels max 5.
const MAX_WELCOME_SCREEN_CHANNELS: usize = 5;

pub fn validate_write(
    writer: &PseudonymKey,
    entry: &GovernanceEntry,
    state: &GovernanceState,
) -> bool {
    // Creator always passes validation
    if state.creator.as_ref() == Some(writer) {
        return true;
    }

    // Banned members can't write valid governance entries
    if state.bans.contains(writer) {
        return false;
    }

    let perms = compute_permissions(writer, None, state, 0);

    match entry {
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. } => has(perms, MANAGE_CHANNELS),

        // Architecture §9.3 line 1946 — a writer can only define/edit a
        // role whose position is strictly less than their own max
        // position. Without this, a mid-rank admin could mint a higher-
        // ranked role and grant it to themselves to climb the hierarchy.
        GovernanceEntry::RoleDefinition { position, .. } => {
            has(perms, MANAGE_ROLES) && *position < state.member_max_position(writer)
        }

        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => {
            // Self-assignable roles bypass MANAGE_ROLES (existing behavior).
            if can_self_assign_role(writer, target, role_id, state) {
                return true;
            }
            // Otherwise: MANAGE_ROLES required AND the role being granted
            // must rank strictly below the writer (architecture §9.3).
            if !has(perms, MANAGE_ROLES) {
                return false;
            }
            let role_position = state.roles.get(role_id).map(|r| r.position).unwrap_or(0);
            role_position < state.member_max_position(writer)
        }

        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => {
            // M10.2 — OWNER protection: the creator's roles can only be
            // unassigned by the creator themselves. Defense-in-depth on
            // top of the rank check; even if an attacker somehow climbs
            // above the creator's rank, this still blocks them.
            if state.creator.as_ref() == Some(target) && writer != target {
                return false;
            }
            // Self-unassign always allowed (you can step down).
            if writer == target {
                return true;
            }
            // Self-assignable role being yanked from the holder by the
            // holder is the self-unassign case above; another peer
            // pulling it falls through to the standard hierarchy check.
            if can_self_assign_role(writer, target, role_id, state) {
                return true;
            }
            if !has(perms, MANAGE_ROLES) {
                return false;
            }
            // Architecture §9.3 — target's max position must be strictly
            // less than writer's. Equal-rank or higher-rank target is
            // immune.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        GovernanceEntry::BanEntry { target, .. } => {
            if !has(perms, BAN_MEMBERS) {
                return false;
            }
            // M10.2 — creator cannot be banned by anyone.
            if state.creator.as_ref() == Some(target) {
                return false;
            }
            // Architecture §9.3 — rank-strict gating.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        // Unbans don't carry a hierarchy concern: banned members hold no
        // active roles inside the community (they're outside it). Anyone
        // with BAN_MEMBERS can lift a ban.
        GovernanceEntry::UnbanEntry { .. } => has(perms, BAN_MEMBERS),

        GovernanceEntry::TimeoutEntry { target, .. } => {
            if !has(perms, TIMEOUT_MEMBERS) {
                return false;
            }
            // M10.2 — creator cannot be timed out by anyone.
            if state.creator.as_ref() == Some(target) {
                return false;
            }
            // Architecture §9.3 — rank-strict gating.
            state.member_max_position(target) < state.member_max_position(writer)
        }

        GovernanceEntry::RemoveTimeoutEntry { .. } => has(perms, TIMEOUT_MEMBERS),

        GovernanceEntry::CommunityMeta { .. } => has(perms, MANAGE_COMMUNITY),

        // Notification default (architecture §17.1 tier 1) is admin-only;
        // any member's local override still trumps it via the resolver.
        GovernanceEntry::CommunityNotificationDefault { level, .. } => {
            matches!(level.as_str(), "all" | "mentions" | "nothing") && has(perms, MANAGE_COMMUNITY)
        }

        // MEK generation bumps use Max-Register (highest generation wins).
        // Rotator authority is verified by checking trigger_departed + cascade_skipped
        // against the deterministic rotator selection algorithm. However, since the
        // merge engine already enforces Max-Register (only highest generation survives),
        // a rogue bump to generation N is superseded by the legitimate bump to N+1.
        // Full rotator verification requires cross-referencing presence timestamps
        // (for cascade_skipped validation), which is done at the sync layer, not here.
        // At the governance CRDT layer, we enforce: writer is not banned (checked above).
        GovernanceEntry::MEKGenerationBump { .. } => true,

        GovernanceEntry::CategoryCreated { .. } | GovernanceEntry::CategoryArchived { .. } => {
            has(perms, MANAGE_CHANNELS)
        }

        GovernanceEntry::PermissionOverwrite { .. } => {
            has(perms, MANAGE_CHANNELS) || has(perms, MANAGE_ROLES)
        }

        GovernanceEntry::ThreadCreated {
            parent_channel_id,
            thread_type,
            invited,
            forum_tag,
            ..
        } => validate_thread_create(
            writer,
            *parent_channel_id,
            thread_type,
            invited,
            forum_tag.as_deref(),
            state,
        ),

        GovernanceEntry::ThreadArchived { .. } => has(perms, MANAGE_THREADS),

        GovernanceEntry::EventCreated { .. } => has(perms, CREATE_EVENTS),

        GovernanceEntry::EventArchived { .. } => has(perms, MANAGE_EVENTS),

        GovernanceEntry::ExpressionAdded {
            kind,
            animated,
            sound_meta,
            ..
        } => {
            (has(perms, MANAGE_EXPRESSIONS) || has(perms, CREATE_EXPRESSIONS))
                && expression_within_limits(kind, *animated, state)
                && soundboard_meta_valid(kind, sound_meta.as_ref())
        }

        GovernanceEntry::ExpressionRemoved { .. } => has(perms, MANAGE_EXPRESSIONS),

        GovernanceEntry::OnboardingConfig {
            mode,
            questions,
            welcome_message,
            guide_steps,
            ..
        } => {
            has(perms, MANAGE_COMMUNITY)
                && onboarding_within_limits(
                    mode,
                    questions,
                    welcome_message.as_deref(),
                    guide_steps,
                )
        }

        GovernanceEntry::WelcomeScreen { channels, .. } => {
            has(perms, MANAGE_COMMUNITY) && channels.len() <= MAX_WELCOME_SCREEN_CHANNELS
        }

        GovernanceEntry::AdminDelete { .. } => has(perms, MANAGE_MESSAGES),

        // Lost Cargo: pinning a file requires MANAGE_COMMUNITY (admin-only)
        // per architecture §28.9 line 3283.
        GovernanceEntry::AttachmentPinned { .. } => has(perms, MANAGE_COMMUNITY),

        // Community-wide policy (notification default + raid thresholds):
        // architecture §17.1 + §20.6 — admin-only.
        GovernanceEntry::CommunityPolicy { .. } => has(perms, MANAGE_COMMUNITY),

        // Segment expansion requires admin-level access
        GovernanceEntry::SegmentAdded { .. } => has(perms, MANAGE_COMMUNITY),

        // Plate Gate lazy channel records (architecture §15.4): any member
        // with SEND_MESSAGES can announce a new channel-segment record —
        // they're the one creating the SMPL record and writing the first
        // message into it. Reader-validates: peers reject ChannelSegmentLinked
        // entries from members without channel write access, and reject
        // entries that name a channel that doesn't exist in governance state.
        GovernanceEntry::ChannelSegmentLinked { channel_id, .. } => {
            has(perms, SEND_MESSAGES) && state.channels.contains_key(channel_id)
        }

        GovernanceEntry::AutoModRule {
            rule_id,
            enabled,
            trigger_json,
            action,
            ..
        } => {
            has(perms, MANAGE_COMMUNITY)
                && validate_automod_rule(rule_id, *enabled, trigger_json, action, state)
        }

        GovernanceEntry::RoleArchived { .. } => has(perms, MANAGE_ROLES),

        GovernanceEntry::CategoryUpdated { .. } => has(perms, MANAGE_CHANNELS),

        // M10.3 — invite minting is reader-validated against three caps:
        //   1. CREATE_INVITES permission (existing).
        //   2. `max_uses` ≤ MAX_USES_PER_INVITE — bound per-invite reuse.
        //   3. Active invites by this writer < policy.max_joins_per_interval —
        //      bound the number of simultaneous join-points opened.
        // A rogue admin who exceeds either cap has their entry silently
        // excluded from the merged state on every honest peer; the joiner
        // who tries to use that invite finds nothing in `state.invites` and
        // reports "invalid invite" via the existing flow.
        GovernanceEntry::InviteCreated { max_uses, .. } => {
            has(perms, CREATE_INVITES)
                && crate::invite_quota::check_max_uses_cap(*max_uses)
                && crate::invite_quota::check_active_invites_cap(state, writer)
        }

        GovernanceEntry::InviteRevoked { .. } => has(perms, MANAGE_COMMUNITY),
    }
}

/// Check if a permission bitmask includes the required permission.
/// ADMINISTRATOR always passes.
fn has(perms: u64, required: u64) -> bool {
    (perms & ADMINISTRATOR != 0) || (perms & required == required)
}

fn can_self_assign_role(
    writer: &PseudonymKey,
    target: &PseudonymKey,
    role_id: &rekindle_types::id::RoleId,
    state: &GovernanceState,
) -> bool {
    writer == target
        && state
            .roles
            .get(role_id)
            .map(|role| role.self_assignable)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests;
