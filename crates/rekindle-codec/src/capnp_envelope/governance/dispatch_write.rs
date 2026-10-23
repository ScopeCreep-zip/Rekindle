//! `governance` write dispatcher.
//!
//! One exhaustive match, no wildcard arm — adding a `GovernanceEntry`
//! variant without a write arm here is a compile error, matching
//! `ControlPayload`'s encode/decode completeness guarantee.

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
    write_event_archived, write_event_created, write_event_rsvp, write_thread_archived,
    write_thread_created, write_thread_unarchived,
};
use super::expression::{
    write_attachment_pinned, write_expression_added, write_expression_removed,
};
use super::membership::{write_member_joined, write_member_record_moved, write_member_removed};
use super::moderation::{
    write_admin_delete, write_auto_mod_rule, write_ban_entry, write_member_approved,
    write_member_rejected, write_mod_report, write_remove_timeout_entry, write_timeout_entry,
    write_unban_entry,
};
use super::onboarding::{write_onboarding_config, write_welcome_screen};
use super::roles::{
    write_role_archived, write_role_assignment, write_role_definition, write_role_unassignment,
};

pub(in crate::capnp_envelope) fn write_governance_entry(
    b: schema::Builder<'_>,
    e: &GovernanceEntry,
) {
    let mut b = b;
    match e {
        GovernanceEntry::AdminDelete {
            message_id,
            channel_id,
            reason,
        } => write_admin_delete(
            b.reborrow().init_admin_delete(),
            message_id,
            *channel_id,
            reason.as_deref(),
        ),
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
        } => write_attachment_pinned(
            b.reborrow().init_attachment_pinned(),
            attachment_id,
            *pinned,
        ),
        GovernanceEntry::AutoModRule { .. } => {
            write_auto_mod_rule(b.reborrow().init_auto_mod_rule(), e);
        }
        GovernanceEntry::BanEntry { target, reason } => {
            write_ban_entry(b.reborrow().init_ban_entry(), target, reason.as_deref());
        }
        GovernanceEntry::CategoryArchived { category_id } => {
            write_category_archived(b.reborrow().init_category_archived(), *category_id);
        }
        GovernanceEntry::CategoryCreated {
            category_id,
            name,
            position,
        } => write_category_created(
            b.reborrow().init_category_created(),
            *category_id,
            name,
            *position,
        ),
        GovernanceEntry::CategoryUpdated {
            category_id,
            name,
            position,
        } => write_category_updated(
            b.reborrow().init_category_updated(),
            *category_id,
            name.as_deref(),
            *position,
        ),
        GovernanceEntry::ChannelArchived { channel_id } => {
            write_channel_archived(b.reborrow().init_channel_archived(), *channel_id);
        }
        GovernanceEntry::ChannelCreated { .. } => {
            write_channel_created(b.reborrow().init_channel_created(), e);
        }
        GovernanceEntry::ChannelSegmentLinked {
            channel_id,
            segment_index,
            record_key,
        } => write_channel_segment_linked(
            b.reborrow().init_channel_segment_linked(),
            *channel_id,
            *segment_index,
            record_key,
        ),
        GovernanceEntry::ChannelUpdated { .. } => {
            write_channel_updated(b.reborrow().init_channel_updated(), e);
        }
        GovernanceEntry::CommunityMeta { .. } => {
            write_community_meta(b.reborrow().init_community_meta(), e);
        }
        GovernanceEntry::CommunityNotificationDefault { level } => {
            write_community_notification_default(
                b.reborrow().init_community_notification_default(),
                level,
            );
        }
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
        } => write_community_policy(
            b.reborrow().init_community_policy(),
            policy_text.as_deref(),
            *max_joins_per_interval,
            *join_interval_seconds,
        ),
        GovernanceEntry::EventArchived { event_id } => {
            write_event_archived(b.reborrow().init_event_archived(), *event_id);
        }
        GovernanceEntry::EventRsvp { event_id, status } => {
            write_event_rsvp(b.reborrow().init_event_rsvp(), *event_id, *status);
        }
        GovernanceEntry::ModReport {
            report_id,
            target,
            reason,
        } => write_mod_report(b.reborrow().init_mod_report(), report_id, target, reason),
        GovernanceEntry::EventCreated { .. } => {
            write_event_created(b.reborrow().init_event_created(), e);
        }
        GovernanceEntry::ExpressionAdded { .. } => {
            write_expression_added(b.reborrow().init_expression_added(), e);
        }
        GovernanceEntry::ExpressionRemoved { expression_id } => {
            write_expression_removed(b.reborrow().init_expression_removed(), expression_id);
        }
        GovernanceEntry::InviteCreated { .. } => {
            write_invite_created(b.reborrow().init_invite_created(), e);
        }
        GovernanceEntry::InviteRevoked { invite_id } => {
            write_invite_revoked(b.reborrow().init_invite_revoked(), invite_id);
        }
        GovernanceEntry::MEKGenerationBump {
            generation,
            trigger_departed,
            cascade_skipped,
        } => write_mek_generation_bump(
            b.reborrow().init_mek_generation_bump(),
            *generation,
            trigger_departed,
            cascade_skipped,
        ),
        GovernanceEntry::OnboardingConfig { .. } => {
            write_onboarding_config(b.reborrow().init_onboarding_config(), e);
        }
        GovernanceEntry::PermissionOverwrite { .. } => {
            write_permission_overwrite(b.reborrow().init_permission_overwrite(), e);
        }
        GovernanceEntry::RemoveTimeoutEntry { target } => {
            write_remove_timeout_entry(b.reborrow().init_remove_timeout_entry(), target);
        }
        GovernanceEntry::RoleArchived { role_id } => {
            write_role_archived(b.reborrow().init_role_archived(), *role_id);
        }
        GovernanceEntry::RoleAssignment { target, role_id } => {
            write_role_assignment(b.reborrow().init_role_assignment(), target, *role_id);
        }
        GovernanceEntry::RoleDefinition { .. } => {
            write_role_definition(b.reborrow().init_role_definition(), e);
        }
        GovernanceEntry::RoleUnassignment { target, role_id } => {
            write_role_unassignment(b.reborrow().init_role_unassignment(), target, *role_id);
        }
        GovernanceEntry::SegmentAdded { .. } => {
            write_segment_added(b.reborrow().init_segment_added(), e);
        }
        GovernanceEntry::ThreadArchived { thread_id } => {
            write_thread_archived(b.reborrow().init_thread_archived(), *thread_id);
        }
        GovernanceEntry::ThreadUnarchived { thread_id } => {
            write_thread_unarchived(b.reborrow().init_thread_unarchived(), *thread_id);
        }
        GovernanceEntry::ThreadCreated { .. } => {
            write_thread_created(b.reborrow().init_thread_created(), e);
        }
        GovernanceEntry::TimeoutEntry { .. } => {
            write_timeout_entry(b.reborrow().init_timeout_entry(), e);
        }
        GovernanceEntry::UnbanEntry { target } => {
            write_unban_entry(b.reborrow().init_unban_entry(), target);
        }
        GovernanceEntry::MemberJoined {
            member_record,
            member_record_owner,
            invite_proof,
        } => write_member_joined(
            b.reborrow().init_member_joined(),
            member_record,
            member_record_owner,
            invite_proof.as_ref(),
        ),
        GovernanceEntry::MemberRecordMoved {
            member_record,
            member_record_owner,
        } => write_member_record_moved(
            b.reborrow().init_member_record_moved(),
            member_record,
            member_record_owner,
        ),
        GovernanceEntry::MemberLeft {} => {
            b.reborrow().init_member_left();
        }
        GovernanceEntry::MemberRemoved { target, reason } => write_member_removed(
            b.reborrow().init_member_removed(),
            target,
            reason.as_deref(),
        ),
        GovernanceEntry::MemberApproved { target } => {
            write_member_approved(b.reborrow().init_member_approved(), target);
        }
        GovernanceEntry::MemberRejected { target, reason } => write_member_rejected(
            b.reborrow().init_member_rejected(),
            target,
            reason.as_deref(),
        ),
        GovernanceEntry::WelcomeScreen {
            description,
            channels,
        } => write_welcome_screen(b.reborrow().init_welcome_screen(), description, channels),
    }
}
