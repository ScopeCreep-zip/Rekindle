//! `canonical_state` (plan E3.2 S7c.3, part (1) of `stateRoot`): every
//! field of [`GovernanceState`], each a section of the canonical encoding
//! ([`super::canon`]). The state and every sub-state are destructured
//! exhaustively, so a new field cannot compile without its section, and
//! every stamp LWW compares is encoded with its row.

use crate::state::{
    AdminDeleteState, AdmissionDecision, AutoModRuleState, CategoryState, ChannelState,
    CommunityPolicyState, EventState, ExpressionState, GovernanceState, InviteState, MemberState,
    MetadataState, ModReportState, NotificationDefaultState, OnboardingState, OverwriteState,
    PendingMemberState, RoleState, RsvpState, SegmentState, ThreadState, TimeoutState,
    WelcomeScreenState,
};

use super::canon::{Canon, Json};

/// Encode each argument in turn.
macro_rules! put {
    ($out:expr; $($v:expr),* $(,)?) => {{ $( Canon::put($v, $out); )* }};
}

/// The canonical encoding of every field of `state`.
#[must_use]
pub fn canonical_state(state: &GovernanceState) -> Vec<u8> {
    let GovernanceState {
        channels,
        roles,
        role_assignments,
        bans,
        members,
        op_log_records,
        departed_heads,
        removed,
        removed_by,
        removal_reasons,
        pending_members,
        admitted,
        admission_mode,
        timeouts,
        mek_generation,
        metadata,
        notification_default,
        categories,
        overwrites,
        threads,
        events,
        event_rsvps,
        expressions,
        admin_deletes,
        pinned_attachments,
        attachment_pin_stamps,
        creators,
        automod_rules,
        onboarding,
        welcome_screen,
        ban_stamps,
        segments,
        invites,
        expression_remove_stamps,
        community_policy,
        mod_reports,
    } = state;
    let out = &mut Vec::new();
    // The roster sections first, in D15's original order.
    put!(out; members, roles, role_assignments);
    put!(out; channels, bans, op_log_records, departed_heads, removed, removed_by);
    put!(out; removal_reasons, pending_members, admitted, &Json(admission_mode));
    put!(out; timeouts, mek_generation, metadata, notification_default, categories);
    put!(out; overwrites, threads, events, event_rsvps, expressions, admin_deletes);
    put!(out; pinned_attachments, attachment_pin_stamps, creators, automod_rules);
    put!(out; onboarding, welcome_screen, ban_stamps, segments);
    put!(out; invites, expression_remove_stamps, community_policy, mod_reports);
    std::mem::take(out)
}

impl Canon for MemberState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            member_record,
            member_record_owner,
            joined,
        } = self;
        put!(out; member_record, member_record_owner, joined);
    }
}

impl Canon for RoleState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            permissions,
            position,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
            stamp,
        } = self;
        put!(out; name, permissions, position, color, hoist, mentionable);
        put!(out; self_assignable, exclusion_group, stamp);
    }
}

impl Canon for ChannelState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            channel_type,
            category_id,
            position,
            topic,
            forum_tags,
            slowmode_seconds,
            nsfw,
            parent_voice_channel_id,
            created,
        } = self;
        put!(out; name, channel_type, category_id, position, topic, forum_tags);
        put!(out; slowmode_seconds, nsfw, parent_voice_channel_id, created);
    }
}

impl Canon for PendingMemberState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            member_record,
            member_record_owner,
            has_invite,
            stamp,
        } = self;
        put!(out; member_record, member_record_owner, has_invite, stamp);
    }
}

impl Canon for AdmissionDecision {
    fn put(&self, out: &mut Vec<u8>) {
        let Self { approved, stamp } = self;
        put!(out; approved, stamp);
    }
}

impl Canon for AdminDeleteState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            by,
            channel_id,
            author,
        } = self;
        put!(out; by, channel_id, author);
    }
}

impl Canon for TimeoutState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            duration_seconds,
            started_at,
            reason,
            stamp,
        } = self;
        put!(out; duration_seconds, started_at, reason, stamp);
    }
}

impl Canon for MetadataState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            description,
            icon_hash,
            banner_hash,
            stamp,
        } = self;
        put!(out; name, description, icon_hash, banner_hash, stamp);
    }
}

impl Canon for NotificationDefaultState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self { level, stamp } = self;
        put!(out; level, stamp);
    }
}

impl Canon for CategoryState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            position,
            created,
        } = self;
        put!(out; name, position, created);
    }
}

impl Canon for OverwriteState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            target_type,
            allow,
            deny,
            stamp,
        } = self;
        put!(out; target_type, allow, deny, stamp);
    }
}

impl Canon for ThreadState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            parent_channel_id,
            name,
            thread_type,
            invited,
            forum_tag,
            auto_archive_seconds,
            creator,
            created,
            archived,
            unarchived,
        } = self;
        put!(out; parent_channel_id, name, thread_type, invited, forum_tag);
        put!(out; auto_archive_seconds, creator, created, archived, unarchived);
    }
}

impl Canon for EventState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            description,
            start_time,
            end_time,
            channel_id,
            cover_image_ref,
            creator_pseudonym,
            recurrence,
            location,
            status,
            stamp,
        } = self;
        put!(out; name, description, start_time, end_time, channel_id, cover_image_ref);
        put!(out; creator_pseudonym, &Json(recurrence), &Json(location), &Json(status), stamp);
    }
}

impl Canon for RsvpState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self { status, stamp } = self;
        put!(out; &Json(status), stamp);
    }
}

impl Canon for ExpressionState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            kind,
            content_hash,
            attachment,
            animated,
            tags,
            sound_meta,
            creator_pseudonym,
            created_at,
            available_to_peers,
            stamp,
        } = self;
        put!(out; name, kind, content_hash, &Json(attachment), animated, tags);
        put!(out; &Json(sound_meta), creator_pseudonym, created_at, available_to_peers, stamp);
    }
}

impl Canon for AutoModRuleState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            name,
            enabled,
            trigger_json,
            action,
            stamp,
        } = self;
        put!(out; name, enabled, trigger_json, &Json(action), stamp);
    }
}

impl Canon for OnboardingState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            enabled,
            mode,
            default_channels,
            questions,
            welcome_message,
            guide_steps,
            stamp,
        } = self;
        put!(out; enabled, mode, default_channels, &Json(questions), welcome_message);
        put!(out; &Json(guide_steps), stamp);
    }
}

impl Canon for WelcomeScreenState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            description,
            channels,
            stamp,
        } = self;
        put!(out; description, &Json(channels), stamp);
    }
}

impl Canon for SegmentState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            segment_index,
            registry_key,
            anchor,
            slot_range_start,
            slot_range_end,
        } = self;
        put!(out; segment_index, registry_key, anchor, slot_range_start, slot_range_end);
    }
}

impl Canon for InviteState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            code_hash,
            max_uses,
            expires_at,
            secrets_record_key,
            created,
            creator_pseudonym,
            uses,
        } = self;
        put!(out; code_hash, max_uses, expires_at, secrets_record_key, created);
        put!(out; creator_pseudonym, uses);
    }
}

impl Canon for CommunityPolicyState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            stamp,
        } = self;
        put!(out; policy_text, max_joins_per_interval, join_interval_seconds, stamp);
    }
}

impl Canon for ModReportState {
    fn put(&self, out: &mut Vec<u8>) {
        let Self {
            reporter,
            target,
            reason,
            stamp,
        } = self;
        put!(out; reporter, &Json(target), reason, stamp);
    }
}
