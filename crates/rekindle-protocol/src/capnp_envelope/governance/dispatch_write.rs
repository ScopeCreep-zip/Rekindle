//! `governance` write dispatcher (router + alphabetical halves).

use crate::community_governance_capnp::governance_entry as schema;
use rekindle_types::governance::GovernanceEntry;

use super::channels::{
    write_category_archived, write_category_created, write_category_updated,
    write_channel_archived, write_channel_created, write_channel_segment_linked,
    write_channel_updated, write_permission_overwrite,
};
use super::community::{
    write_community_meta, write_community_notification_default, write_community_policy,
    write_invite_created, write_invite_revoked, write_mek_generation_bump, write_segment_added,
};
use super::events::{
    write_event_archived, write_event_created, write_thread_archived, write_thread_created,
};
use super::expression::{
    write_attachment_pinned, write_expression_added, write_expression_removed,
};
use super::moderation::{
    write_admin_delete, write_admission_policy, write_auto_mod_rule, write_ban_entry,
    write_join_requested, write_member_approved, write_member_rejected, write_remove_timeout_entry,
    write_timeout_entry, write_unban_entry,
};
use super::onboarding::{write_onboarding_config, write_welcome_screen};
use super::roles::{
    write_role_archived, write_role_assignment, write_role_definition, write_role_unassignment,
};
use super::shared::CategoryUpdate;

pub(in crate::capnp_envelope) fn write_governance_entry(
    b: schema::Builder<'_>,
    e: &GovernanceEntry,
) {
    use GovernanceEntry as G;
    match e {
        G::AdminDelete { .. }
        | G::AttachmentPinned { .. }
        | G::AutoModRule { .. }
        | G::BanEntry { .. }
        | G::CategoryArchived { .. }
        | G::CategoryCreated { .. }
        | G::CategoryUpdated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelCreated { .. }
        | G::ChannelSegmentLinked { .. }
        | G::ChannelUpdated { .. }
        | G::CommunityMeta { .. }
        | G::CommunityNotificationDefault { .. }
        | G::CommunityPolicy { .. }
        | G::EventArchived { .. }
        | G::EventCreated { .. }
        | G::ExpressionAdded { .. } => write_governance_entry_first_half(b, e),
        _ => write_governance_entry_second_half(b, e),
    }
}
fn write_governance_entry_first_half(b: schema::Builder<'_>, e: &GovernanceEntry) {
    let mut b = b;
    match e {
        GovernanceEntry::AdminDelete {
            message_id,
            channel_id,
            reason,
            lamport,
        } => write_admin_delete(
            b.reborrow().init_admin_delete(),
            message_id,
            *channel_id,
            reason.as_deref(),
            *lamport,
        ),
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
            lamport,
        } => write_attachment_pinned(
            b.reborrow().init_attachment_pinned(),
            attachment_id,
            *pinned,
            *lamport,
        ),
        GovernanceEntry::AutoModRule {
            rule_id,
            name,
            enabled,
            trigger_json,
            action,
            lamport,
        } => write_auto_mod_rule(
            b.reborrow().init_auto_mod_rule(),
            rule_id,
            name,
            *enabled,
            trigger_json,
            action,
            *lamport,
        ),
        GovernanceEntry::BanEntry {
            target,
            reason,
            lamport,
        } => write_ban_entry(
            b.reborrow().init_ban_entry(),
            target,
            reason.as_deref(),
            *lamport,
        ),
        GovernanceEntry::CategoryArchived {
            category_id,
            lamport,
        } => write_category_archived(
            b.reborrow().init_category_archived(),
            *category_id,
            *lamport,
        ),
        GovernanceEntry::CategoryCreated {
            category_id,
            name,
            position,
            lamport,
        } => write_category_created(
            b.reborrow().init_category_created(),
            *category_id,
            name,
            *position,
            *lamport,
        ),
        GovernanceEntry::CategoryUpdated {
            category_id,
            name,
            position,
            lamport,
        } => write_category_updated(
            b.reborrow().init_category_updated(),
            *category_id,
            name.as_deref(),
            *position,
            *lamport,
        ),
        GovernanceEntry::ChannelArchived {
            channel_id,
            lamport,
        } => write_channel_archived(b.reborrow().init_channel_archived(), *channel_id, *lamport),
        GovernanceEntry::ChannelCreated { .. } => {
            write_channel_created(b.reborrow().init_channel_created(), e);
        }
        GovernanceEntry::ChannelSegmentLinked {
            channel_id,
            segment_index,
            record_key,
            lamport,
        } => write_channel_segment_linked(
            b.reborrow().init_channel_segment_linked(),
            *channel_id,
            *segment_index,
            record_key,
            *lamport,
        ),
        GovernanceEntry::ChannelUpdated {
            channel_id,
            name,
            topic,
            forum_tags,
            position,
            slowmode_seconds,
            nsfw,
            category_id,
            lamport,
        } => {
            let cat_update = match category_id {
                None => CategoryUpdate::Unchanged,
                Some(None) => CategoryUpdate::Cleared,
                Some(Some(c)) => CategoryUpdate::Set(*c),
            };
            write_channel_updated(
                b.reborrow().init_channel_updated(),
                *channel_id,
                name.as_deref(),
                topic.as_deref(),
                forum_tags.as_deref(),
                *position,
                *slowmode_seconds,
                *nsfw,
                cat_update,
                *lamport,
            );
        }
        GovernanceEntry::CommunityMeta {
            name,
            description,
            icon_hash,
            banner_hash,
            lamport,
        } => write_community_meta(
            b.reborrow().init_community_meta(),
            name.as_deref(),
            description.as_deref(),
            icon_hash.as_deref(),
            banner_hash.as_deref(),
            *lamport,
        ),
        GovernanceEntry::CommunityNotificationDefault { level, lamport } => {
            write_community_notification_default(
                b.reborrow().init_community_notification_default(),
                level,
                *lamport,
            );
        }
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            lamport,
        } => write_community_policy(
            b.reborrow().init_community_policy(),
            policy_text.as_deref(),
            *max_joins_per_interval,
            *join_interval_seconds,
            *lamport,
        ),
        GovernanceEntry::EventArchived { event_id, lamport } => {
            write_event_archived(b.reborrow().init_event_archived(), *event_id, *lamport);
        }
        GovernanceEntry::EventCreated { .. } => {
            write_event_created(b.reborrow().init_event_created(), e);
        }
        GovernanceEntry::ExpressionAdded { .. } => {
            write_expression_added(b.reborrow().init_expression_added(), e);
        }
        _ => unreachable!("write_governance_entry_first_half called with second-half variant"),
    }
}
fn write_governance_entry_second_half(b: schema::Builder<'_>, e: &GovernanceEntry) {
    let mut b = b;
    match e {
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport,
        } => write_expression_removed(
            b.reborrow().init_expression_removed(),
            expression_id,
            *lamport,
        ),
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash,
            max_uses,
            expires_at,
            secrets_record_key,
            lamport,
        } => write_invite_created(
            b.reborrow().init_invite_created(),
            invite_id,
            code_hash,
            *max_uses,
            *expires_at,
            secrets_record_key,
            *lamport,
        ),
        GovernanceEntry::InviteRevoked { invite_id, lamport } => {
            write_invite_revoked(b.reborrow().init_invite_revoked(), invite_id, *lamport);
        }
        GovernanceEntry::MEKGenerationBump {
            generation,
            trigger_departed,
            cascade_skipped,
            lamport,
        } => write_mek_generation_bump(
            b.reborrow().init_mek_generation_bump(),
            *generation,
            trigger_departed,
            cascade_skipped,
            *lamport,
        ),
        GovernanceEntry::OnboardingConfig {
            enabled,
            mode,
            default_channels,
            questions,
            welcome_message,
            guide_steps,
            lamport,
        } => write_onboarding_config(
            b.reborrow().init_onboarding_config(),
            *enabled,
            mode,
            default_channels,
            questions,
            welcome_message.as_deref(),
            guide_steps,
            *lamport,
        ),
        GovernanceEntry::PermissionOverwrite {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
            lamport,
        } => write_permission_overwrite(
            b.reborrow().init_permission_overwrite(),
            *channel_id,
            target_type,
            target_id,
            *allow,
            *deny,
            *lamport,
        ),
        GovernanceEntry::RemoveTimeoutEntry { target, lamport } => {
            write_remove_timeout_entry(b.reborrow().init_remove_timeout_entry(), target, *lamport);
        }
        GovernanceEntry::RoleArchived { role_id, lamport } => {
            write_role_archived(b.reborrow().init_role_archived(), *role_id, *lamport);
        }
        GovernanceEntry::RoleAssignment {
            target,
            role_id,
            lamport,
        } => write_role_assignment(
            b.reborrow().init_role_assignment(),
            target,
            *role_id,
            *lamport,
        ),
        GovernanceEntry::RoleDefinition {
            role_id,
            name,
            permissions,
            position,
            color,
            hoist,
            mentionable,
            self_assignable,
            exclusion_group,
            lamport,
        } => write_role_definition(
            b.reborrow().init_role_definition(),
            *role_id,
            name,
            *permissions,
            *position,
            *color,
            *hoist,
            *mentionable,
            *self_assignable,
            exclusion_group.as_deref(),
            *lamport,
        ),
        GovernanceEntry::RoleUnassignment {
            target,
            role_id,
            lamport,
        } => write_role_unassignment(
            b.reborrow().init_role_unassignment(),
            target,
            *role_id,
            *lamport,
        ),
        GovernanceEntry::SegmentAdded {
            segment_index,
            registry_key,
            governance_key,
            slot_range_start,
            slot_range_end,
            lamport,
        } => write_segment_added(
            b.reborrow().init_segment_added(),
            *segment_index,
            registry_key,
            governance_key,
            *slot_range_start,
            *slot_range_end,
            *lamport,
        ),
        GovernanceEntry::ThreadArchived { thread_id, lamport } => {
            write_thread_archived(b.reborrow().init_thread_archived(), *thread_id, *lamport);
        }
        GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name,
            thread_type,
            record_key,
            invited,
            forum_tag,
            auto_archive_seconds,
            lamport,
        } => write_thread_created(
            b.reborrow().init_thread_created(),
            *thread_id,
            *parent_channel_id,
            name,
            thread_type,
            record_key.as_deref(),
            invited,
            forum_tag.as_deref(),
            *auto_archive_seconds,
            *lamport,
        ),
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            reason,
            started_at,
            lamport,
        } => write_timeout_entry(
            b.reborrow().init_timeout_entry(),
            target,
            *duration_seconds,
            reason.as_deref(),
            *started_at,
            *lamport,
        ),
        GovernanceEntry::UnbanEntry { target, lamport } => {
            write_unban_entry(b.reborrow().init_unban_entry(), target, *lamport);
        }
        GovernanceEntry::JoinRequested {
            requester,
            display_name,
            lamport,
        } => write_join_requested(
            b.reborrow().init_join_requested(),
            requester,
            display_name,
            *lamport,
        ),
        GovernanceEntry::MemberApproved { target, lamport } => {
            write_member_approved(b.reborrow().init_member_approved(), target, *lamport);
        }
        GovernanceEntry::MemberRejected {
            target,
            reason,
            lamport,
        } => write_member_rejected(
            b.reborrow().init_member_rejected(),
            target,
            reason.as_deref(),
            *lamport,
        ),
        GovernanceEntry::AdmissionPolicy { mode, lamport } => {
            write_admission_policy(b.reborrow().init_admission_policy(), *mode, *lamport);
        }
        GovernanceEntry::WelcomeScreen {
            description,
            channels,
            lamport,
        } => write_welcome_screen(
            b.reborrow().init_welcome_screen(),
            description,
            channels,
            *lamport,
        ),
        _ => unreachable!("write_governance_entry_second_half called with first-half variant"),
    }
}
