//! `GovernanceEntry` (34-arm union) encoder / decoder.
//!
//! Mirrors `rekindle_types::governance::GovernanceEntry` against the
//! schema in `schemas/community_governance.capnp`. Each variant has
//! its own `write_<name>` / `read_<name>` helper so the dispatcher
//! `write_governance_entry` / `read_governance_entry` stay below the
//! workspace `too-many-lines` threshold and individual variant logic
//! is independently reviewable.

use crate::capnp_codec::{capnp_err, not_in_schema, pack, text_to_string, unpack};
use crate::community_event_capnp;
use crate::community_governance_capnp::{self as schema_pkg, governance_entry as schema};
use crate::error::ProtocolError;
use rekindle_types::expression::SoundboardMeta;
use rekindle_types::governance::{
    GovernanceEntry, GuideStep, OnboardingOption, OnboardingQuestion, WelcomeChannel,
};
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

use super::len_u32;
use super::sub_types::{
    category_id_from_capnp, channel_id_from_capnp, event_id_from_capnp, pseudonym_key_from_capnp,
    pseudonym_key_to_capnp, read_event_location_via_event_capnp,
    read_recurrence_rule_via_event_capnp, role_id_from_capnp, thread_id_from_capnp,
    uuid16_from_capnp, uuid16_to_capnp, write_event_location_via_event_capnp,
    write_recurrence_rule_via_event_capnp,
};

/// Tri-state for `ChannelUpdated.category_id`. The Rust enum variant
/// uses `Option<Option<CategoryId>>` (clippy `option_option` smells)
/// to distinguish "no change" / "clear category" / "set category", so
/// the encoder converts to this explicit shape at the dispatcher
/// boundary. Conversion is inlined where used to avoid lifting the
/// `Option<Option<...>>` shape into any helper signature.
#[derive(Clone, Copy)]
enum CategoryUpdate {
    Unchanged,
    Cleared,
    Set(CategoryId),
}

// ── Public API ───────────────────────────────────────────────────────

/// Encode a single `GovernanceEntry` to packed Cap'n Proto bytes
/// (used for direct serialization, e.g. SMPL subkey writes).
pub fn encode_governance_entry(entry: &GovernanceEntry) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let root = builder.init_root::<schema::Builder<'_>>();
    write_governance_entry(root, entry);
    pack(&builder)
}

/// Decode a single `GovernanceEntry` from packed Cap'n Proto bytes.
pub fn decode_governance_entry(bytes: &[u8]) -> Result<GovernanceEntry, ProtocolError> {
    let reader = unpack(bytes)?;
    let root = reader
        .get_root::<schema::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read_governance_entry(root)
}

// ── Dispatcher (write) ───────────────────────────────────────────────
//
// Split into two halves to stay below the workspace
// `too-many-lines-threshold = 300`. Each half exhaustively handles its
// own subset of variants and `unreachable!()`s on misuse. The outer
// `write_governance_entry` routes by variant tag. The split is
// alphabetical (AdminDelete..ExpressionAdded vs ExpressionRemoved..
// WelcomeScreen) — there's no semantic grouping, just line-count.

pub(super) fn write_governance_entry(b: schema::Builder<'_>, e: &GovernanceEntry) {
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
            encrypted_secrets,
            lamport,
        } => write_invite_created(
            b.reborrow().init_invite_created(),
            invite_id,
            code_hash,
            *max_uses,
            *expires_at,
            encrypted_secrets,
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
            write_remove_timeout_entry(b.reborrow().init_remove_timeout_entry(), target, *lamport)
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

// ── Dispatcher (read) ────────────────────────────────────────────────

pub(super) fn read_governance_entry(
    r: schema::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    use schema::Which;
    match r.which().map_err(not_in_schema)? {
        Which::ChannelCreated(p) => read_channel_created(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelArchived(p) => read_channel_archived(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelUpdated(p) => read_channel_updated(p.map_err(|e| capnp_err(&e))?),
        Which::RoleDefinition(p) => read_role_definition(p.map_err(|e| capnp_err(&e))?),
        Which::RoleAssignment(p) => read_role_assignment(p.map_err(|e| capnp_err(&e))?),
        Which::RoleUnassignment(p) => read_role_unassignment(p.map_err(|e| capnp_err(&e))?),
        Which::BanEntry(p) => read_ban_entry(p.map_err(|e| capnp_err(&e))?),
        Which::UnbanEntry(p) => read_unban_entry(p.map_err(|e| capnp_err(&e))?),
        Which::TimeoutEntry(p) => read_timeout_entry(p.map_err(|e| capnp_err(&e))?),
        Which::RemoveTimeoutEntry(p) => read_remove_timeout_entry(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityMeta(p) => read_community_meta(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityNotificationDefault(p) => {
            read_community_notification_default(p.map_err(|e| capnp_err(&e))?)
        }
        Which::MekGenerationBump(p) => read_mek_generation_bump(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryCreated(p) => read_category_created(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryArchived(p) => read_category_archived(p.map_err(|e| capnp_err(&e))?),
        Which::PermissionOverwrite(p) => read_permission_overwrite(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadCreated(p) => read_thread_created(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadArchived(p) => read_thread_archived(p.map_err(|e| capnp_err(&e))?),
        Which::EventCreated(p) => read_event_created(p.map_err(|e| capnp_err(&e))?),
        Which::ExpressionAdded(p) => read_expression_added(p.map_err(|e| capnp_err(&e))?),
        Which::ExpressionRemoved(p) => read_expression_removed(p.map_err(|e| capnp_err(&e))?),
        Which::EventArchived(p) => read_event_archived(p.map_err(|e| capnp_err(&e))?),
        Which::OnboardingConfig(p) => read_onboarding_config(p.map_err(|e| capnp_err(&e))?),
        Which::WelcomeScreen(p) => read_welcome_screen(p.map_err(|e| capnp_err(&e))?),
        Which::AdminDelete(p) => read_admin_delete(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelSegmentLinked(p) => {
            read_channel_segment_linked(p.map_err(|e| capnp_err(&e))?)
        }
        Which::SegmentAdded(p) => read_segment_added(p.map_err(|e| capnp_err(&e))?),
        Which::AutoModRule(p) => read_auto_mod_rule(p.map_err(|e| capnp_err(&e))?),
        Which::RoleArchived(p) => read_role_archived(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryUpdated(p) => read_category_updated(p.map_err(|e| capnp_err(&e))?),
        Which::InviteCreated(p) => read_invite_created(p.map_err(|e| capnp_err(&e))?),
        Which::InviteRevoked(p) => read_invite_revoked(p.map_err(|e| capnp_err(&e))?),
        Which::AttachmentPinned(p) => read_attachment_pinned(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityPolicy(p) => read_community_policy(p.map_err(|e| capnp_err(&e))?),
    }
}

// ── Per-variant write helpers ────────────────────────────────────────

fn write_channel_created(
    mut p: schema_pkg::channel_created_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::ChannelCreated {
        channel_id,
        name,
        channel_type,
        record_key,
        category_id,
        position,
        parent_voice_channel_id,
        lamport,
    } = e
    else {
        unreachable!("write_channel_created: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_name(name);
    p.set_channel_type(channel_type);
    p.set_record_key(record_key);
    p.set_has_category_id(category_id.is_some());
    if let Some(c) = category_id {
        uuid16_to_capnp(p.reborrow().init_category_id(), &c.0);
    }
    p.set_position(*position);
    p.set_has_parent_voice_channel_id(parent_voice_channel_id.is_some());
    if let Some(pv) = parent_voice_channel_id {
        uuid16_to_capnp(p.reborrow().init_parent_voice_channel_id(), &pv.0);
    }
    p.set_lamport(*lamport);
}

fn write_channel_archived(
    mut p: schema_pkg::channel_archived_entry::Builder<'_>,
    channel_id: ChannelId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_lamport(lamport);
}

fn write_channel_updated(
    mut p: schema_pkg::channel_updated_entry::Builder<'_>,
    channel_id: ChannelId,
    name: Option<&str>,
    topic: Option<&str>,
    forum_tags: Option<&[String]>,
    position: Option<u32>,
    slowmode_seconds: Option<u32>,
    nsfw: Option<bool>,
    category_id: CategoryUpdate,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_topic(topic.is_some());
    if let Some(t) = topic {
        p.set_topic(t);
    }
    p.set_has_forum_tags(forum_tags.is_some());
    if let Some(tags) = forum_tags {
        let mut list = p.reborrow().init_forum_tags(len_u32(tags.len()));
        for (i, t) in tags.iter().enumerate() {
            list.set(len_u32(i), t.as_str());
        }
    }
    p.set_has_position(position.is_some());
    if let Some(pos) = position {
        p.set_position(pos);
    }
    p.set_has_slowmode_seconds(slowmode_seconds.is_some());
    if let Some(s) = slowmode_seconds {
        p.set_slowmode_seconds(s);
    }
    p.set_has_nsfw(nsfw.is_some());
    if let Some(n) = nsfw {
        p.set_nsfw(n);
    }
    match category_id {
        CategoryUpdate::Unchanged => {
            p.set_has_category_id(false);
        }
        CategoryUpdate::Cleared => {
            p.set_has_category_id(true);
            p.set_category_id_present(false);
        }
        CategoryUpdate::Set(c) => {
            p.set_has_category_id(true);
            p.set_category_id_present(true);
            uuid16_to_capnp(p.reborrow().init_category_id(), &c.0);
        }
    }
    p.set_lamport(lamport);
}

fn write_role_definition(
    mut p: schema_pkg::role_definition_entry::Builder<'_>,
    role_id: RoleId,
    name: &str,
    permissions: u64,
    position: u32,
    color: u32,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<&str>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_name(name);
    p.set_permissions(permissions);
    p.set_position(position);
    p.set_color(color);
    p.set_hoist(hoist);
    p.set_mentionable(mentionable);
    p.set_self_assignable(self_assignable);
    p.set_has_exclusion_group(exclusion_group.is_some());
    if let Some(g) = exclusion_group {
        p.set_exclusion_group(g);
    }
    p.set_lamport(lamport);
}

fn write_role_assignment(
    mut p: schema_pkg::role_assignment_entry::Builder<'_>,
    target: &PseudonymKey,
    role_id: RoleId,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

fn write_role_unassignment(
    mut p: schema_pkg::role_unassignment_entry::Builder<'_>,
    target: &PseudonymKey,
    role_id: RoleId,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

fn write_ban_entry(
    mut p: schema_pkg::ban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_lamport(lamport);
}

fn write_unban_entry(
    mut p: schema_pkg::unban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_lamport(lamport);
}

fn write_timeout_entry(
    mut p: schema_pkg::timeout_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    duration_seconds: u64,
    reason: Option<&str>,
    started_at: u64,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_duration_seconds(duration_seconds);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_started_at(started_at);
    p.set_lamport(lamport);
}

fn write_remove_timeout_entry(
    mut p: schema_pkg::remove_timeout_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_lamport(lamport);
}

fn write_community_meta(
    mut p: schema_pkg::community_meta_entry::Builder<'_>,
    name: Option<&str>,
    description: Option<&str>,
    icon_hash: Option<&str>,
    banner_hash: Option<&str>,
    lamport: u64,
) {
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_description(description.is_some());
    if let Some(d) = description {
        p.set_description(d);
    }
    p.set_has_icon_hash(icon_hash.is_some());
    if let Some(h) = icon_hash {
        p.set_icon_hash(h);
    }
    p.set_has_banner_hash(banner_hash.is_some());
    if let Some(h) = banner_hash {
        p.set_banner_hash(h);
    }
    p.set_lamport(lamport);
}

fn write_community_notification_default(
    mut p: schema_pkg::community_notification_default_entry::Builder<'_>,
    level: &str,
    lamport: u64,
) {
    p.set_level(level);
    p.set_lamport(lamport);
}

fn write_mek_generation_bump(
    mut p: schema_pkg::m_e_k_generation_bump_entry::Builder<'_>,
    generation: u64,
    trigger_departed: &PseudonymKey,
    cascade_skipped: &[PseudonymKey],
    lamport: u64,
) {
    p.set_generation(generation);
    pseudonym_key_to_capnp(p.reborrow().init_trigger_departed(), trigger_departed);
    let mut list = p
        .reborrow()
        .init_cascade_skipped(len_u32(cascade_skipped.len()));
    for (i, k) in cascade_skipped.iter().enumerate() {
        pseudonym_key_to_capnp(list.reborrow().get(len_u32(i)), k);
    }
    p.set_lamport(lamport);
}

fn write_category_created(
    mut p: schema_pkg::category_created_entry::Builder<'_>,
    category_id: CategoryId,
    name: &str,
    position: u32,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_name(name);
    p.set_position(position);
    p.set_lamport(lamport);
}

fn write_category_archived(
    mut p: schema_pkg::category_archived_entry::Builder<'_>,
    category_id: CategoryId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_lamport(lamport);
}

fn write_permission_overwrite(
    mut p: schema_pkg::permission_overwrite_entry::Builder<'_>,
    channel_id: ChannelId,
    target_type: &str,
    target_id: &str,
    allow: u64,
    deny: u64,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_target_type(target_type);
    p.set_target_id(target_id);
    p.set_allow(allow);
    p.set_deny(deny);
    p.set_lamport(lamport);
}

fn write_thread_created(
    mut p: schema_pkg::thread_created_entry::Builder<'_>,
    thread_id: ThreadId,
    parent_channel_id: ChannelId,
    name: &str,
    thread_type: &str,
    record_key: Option<&str>,
    invited: &[PseudonymKey],
    forum_tag: Option<&str>,
    auto_archive_seconds: u64,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_thread_id(), &thread_id.0);
    uuid16_to_capnp(p.reborrow().init_parent_channel_id(), &parent_channel_id.0);
    p.set_name(name);
    p.set_thread_type(thread_type);
    p.set_has_record_key(record_key.is_some());
    if let Some(k) = record_key {
        p.set_record_key(k);
    }
    let mut inv_list = p.reborrow().init_invited(len_u32(invited.len()));
    for (i, k) in invited.iter().enumerate() {
        pseudonym_key_to_capnp(inv_list.reborrow().get(len_u32(i)), k);
    }
    p.set_has_forum_tag(forum_tag.is_some());
    if let Some(t) = forum_tag {
        p.set_forum_tag(t);
    }
    p.set_auto_archive_seconds(auto_archive_seconds);
    p.set_lamport(lamport);
}

fn write_thread_archived(
    mut p: schema_pkg::thread_archived_entry::Builder<'_>,
    thread_id: ThreadId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_thread_id(), &thread_id.0);
    p.set_lamport(lamport);
}

// `write_event_created` and `write_expression_added` re-extract their
// fields via `let ... else { unreachable!() }` so the helper signature
// stays under the `too_many_arguments` threshold without needing
// `#[allow]`. Matches the per-variant pattern in `control.rs`.
fn write_event_created(mut p: schema_pkg::event_created_entry::Builder<'_>, e: &GovernanceEntry) {
    let GovernanceEntry::EventCreated {
        event_id,
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
        lamport,
    } = e
    else {
        unreachable!("write_event_created: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_event_id(), &event_id.0);
    p.set_name(name);
    p.set_has_description(description.is_some());
    if let Some(d) = description {
        p.set_description(d);
    }
    p.set_start_time(*start_time);
    p.set_has_end_time(end_time.is_some());
    if let Some(t) = end_time {
        p.set_end_time(*t);
    }
    p.set_has_channel_id(channel_id.is_some());
    if let Some(c) = channel_id {
        uuid16_to_capnp(p.reborrow().init_channel_id(), &c.0);
    }
    p.set_has_cover_image_ref(cover_image_ref.is_some());
    if let Some(r) = cover_image_ref {
        p.set_cover_image_ref(r);
    }
    p.set_has_creator_pseudonym(creator_pseudonym.is_some());
    if let Some(c) = creator_pseudonym {
        pseudonym_key_to_capnp(p.reborrow().init_creator_pseudonym(), c);
    }
    p.set_has_recurrence(recurrence.is_some());
    if let Some(r) = recurrence {
        write_recurrence_rule_via_event_capnp(p.reborrow().init_recurrence(), r);
    }
    p.set_has_location(location.is_some());
    if let Some(loc) = location {
        write_event_location_via_event_capnp(p.reborrow().init_location(), loc);
    }
    p.set_has_status(status.is_some());
    if let Some(s) = status {
        p.set_status(event_status_to_capnp(*s));
    }
    p.set_lamport(*lamport);
}

fn write_expression_added(
    mut p: schema_pkg::expression_added_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::ExpressionAdded {
        expression_id,
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
        lamport,
    } = e
    else {
        unreachable!("write_expression_added: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_expression_id(), expression_id);
    p.set_name(name);
    p.set_kind(kind);
    p.set_content_hash(content_hash);
    // Architecture §18.4 — encoders never write the deprecated
    // inline_data path. Bytes travel via the AttachmentOffer + Lost Cargo.
    p.set_has_inline_data(false);
    p.set_animated(*animated);
    let mut tag_list = p.reborrow().init_tags(len_u32(tags.len()));
    for (i, t) in tags.iter().enumerate() {
        tag_list.set(len_u32(i), t.as_str());
    }
    p.set_has_sound_meta(sound_meta.is_some());
    if let Some(sm) = sound_meta {
        write_sound_meta(p.reborrow().init_sound_meta(), sm);
    }
    p.set_has_creator_pseudonym(creator_pseudonym.is_some());
    if let Some(c) = creator_pseudonym {
        pseudonym_key_to_capnp(p.reborrow().init_creator_pseudonym(), c);
    }
    p.set_has_created_at(created_at.is_some());
    if let Some(t) = created_at {
        p.set_created_at(*t);
    }
    p.set_has_available_to_peers(available_to_peers.is_some());
    if let Some(a) = available_to_peers {
        p.set_available_to_peers(*a);
    }
    p.set_lamport(*lamport);
    p.set_has_attachment(attachment.is_some());
    if let Some(offer) = attachment {
        write_attachment_offer(p.reborrow().init_attachment(), offer);
    }
}

fn write_attachment_offer(
    mut p: schema_pkg::attachment_offer::Builder<'_>,
    offer: &rekindle_types::attachment::AttachmentOffer,
) {
    uuid16_to_capnp(p.reborrow().init_attachment_id(), &offer.attachment_id);
    p.set_filename(&offer.filename);
    p.set_mime_type(&offer.mime_type);
    p.set_total_size(offer.total_size);
    p.set_chunk_count(offer.chunk_count);
    p.set_chunk_size(offer.chunk_size);
    p.set_merkle_root(&offer.merkle_root);
    let mut hashes = p
        .reborrow()
        .init_chunk_hashes(len_u32(offer.chunk_hashes.len()));
    for (i, h) in offer.chunk_hashes.iter().enumerate() {
        hashes.set(len_u32(i), h);
    }
    p.set_wrapped_fek(&offer.wrapped_fek);
    p.set_fek_mek_generation(offer.fek_mek_generation);
}

fn write_expression_removed(
    mut p: schema_pkg::expression_removed_entry::Builder<'_>,
    expression_id: &[u8; 16],
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_expression_id(), expression_id);
    p.set_lamport(lamport);
}

fn write_event_archived(
    mut p: schema_pkg::event_archived_entry::Builder<'_>,
    event_id: EventId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_event_id(), &event_id.0);
    p.set_lamport(lamport);
}

fn write_onboarding_config(
    mut p: schema_pkg::onboarding_config_entry::Builder<'_>,
    enabled: bool,
    mode: &str,
    default_channels: &[ChannelId],
    questions: &[OnboardingQuestion],
    welcome_message: Option<&str>,
    guide_steps: &[GuideStep],
    lamport: u64,
) {
    p.set_enabled(enabled);
    p.set_mode(mode);
    let mut chans = p
        .reborrow()
        .init_default_channels(len_u32(default_channels.len()));
    for (i, c) in default_channels.iter().enumerate() {
        uuid16_to_capnp(chans.reborrow().get(len_u32(i)), &c.0);
    }
    let mut q_list = p.reborrow().init_questions(len_u32(questions.len()));
    for (i, q) in questions.iter().enumerate() {
        write_onboarding_question(q_list.reborrow().get(len_u32(i)), q);
    }
    p.set_has_welcome_message(welcome_message.is_some());
    if let Some(m) = welcome_message {
        p.set_welcome_message(m);
    }
    let mut steps = p.reborrow().init_guide_steps(len_u32(guide_steps.len()));
    for (i, s) in guide_steps.iter().enumerate() {
        write_guide_step(steps.reborrow().get(len_u32(i)), s);
    }
    p.set_lamport(lamport);
}

fn write_welcome_screen(
    mut p: schema_pkg::welcome_screen_entry::Builder<'_>,
    description: &str,
    channels: &[WelcomeChannel],
    lamport: u64,
) {
    p.set_description(description);
    let mut list = p.reborrow().init_channels(len_u32(channels.len()));
    for (i, c) in channels.iter().enumerate() {
        write_welcome_channel(list.reborrow().get(len_u32(i)), c);
    }
    p.set_lamport(lamport);
}

fn write_admin_delete(
    mut p: schema_pkg::admin_delete_entry::Builder<'_>,
    message_id: &[u8; 16],
    channel_id: ChannelId,
    reason: Option<&str>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_message_id(), message_id);
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_lamport(lamport);
}

fn write_channel_segment_linked(
    mut p: schema_pkg::channel_segment_linked_entry::Builder<'_>,
    channel_id: ChannelId,
    segment_index: u32,
    record_key: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_segment_index(segment_index);
    p.set_record_key(record_key);
    p.set_lamport(lamport);
}

fn write_segment_added(
    mut p: schema_pkg::segment_added_entry::Builder<'_>,
    segment_index: u32,
    registry_key: &str,
    governance_key: &str,
    slot_range_start: u32,
    slot_range_end: u32,
    lamport: u64,
) {
    p.set_segment_index(segment_index);
    p.set_registry_key(registry_key);
    p.set_governance_key(governance_key);
    p.set_slot_range_start(slot_range_start);
    p.set_slot_range_end(slot_range_end);
    p.set_lamport(lamport);
}

fn write_auto_mod_rule(
    mut p: schema_pkg::auto_mod_rule_entry::Builder<'_>,
    rule_id: &[u8; 16],
    name: &str,
    enabled: bool,
    trigger_json: &str,
    action: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_rule_id(), rule_id);
    p.set_name(name);
    p.set_enabled(enabled);
    p.set_trigger_json(trigger_json);
    p.set_action(action);
    p.set_lamport(lamport);
}

fn write_role_archived(
    mut p: schema_pkg::role_archived_entry::Builder<'_>,
    role_id: RoleId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

fn write_category_updated(
    mut p: schema_pkg::category_updated_entry::Builder<'_>,
    category_id: CategoryId,
    name: Option<&str>,
    position: Option<u32>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_category_id(), &category_id.0);
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_position(position.is_some());
    if let Some(pos) = position {
        p.set_position(pos);
    }
    p.set_lamport(lamport);
}

fn write_invite_created(
    mut p: schema_pkg::invite_created_entry::Builder<'_>,
    invite_id: &[u8; 16],
    code_hash: &str,
    max_uses: u32,
    expires_at: Option<u64>,
    encrypted_secrets: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_invite_id(), invite_id);
    p.set_code_hash(code_hash);
    p.set_max_uses(max_uses);
    p.set_has_expires_at(expires_at.is_some());
    if let Some(t) = expires_at {
        p.set_expires_at(t);
    }
    p.set_encrypted_secrets(encrypted_secrets);
    p.set_lamport(lamport);
}

fn write_invite_revoked(
    mut p: schema_pkg::invite_revoked_entry::Builder<'_>,
    invite_id: &[u8; 16],
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_invite_id(), invite_id);
    p.set_lamport(lamport);
}

fn write_attachment_pinned(
    mut p: schema_pkg::attachment_pinned_entry::Builder<'_>,
    attachment_id: &[u8; 16],
    pinned: bool,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_attachment_id(), attachment_id);
    p.set_pinned(pinned);
    p.set_lamport(lamport);
}

fn write_community_policy(
    mut p: schema_pkg::community_policy_entry::Builder<'_>,
    policy_text: Option<&str>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
    lamport: u64,
) {
    p.set_has_policy_text(policy_text.is_some());
    if let Some(t) = policy_text {
        p.set_policy_text(t);
    }
    p.set_max_joins_per_interval(max_joins_per_interval);
    p.set_join_interval_seconds(join_interval_seconds);
    p.set_lamport(lamport);
}

// ── Per-variant read helpers ─────────────────────────────────────────

fn read_channel_created(
    p: schema_pkg::channel_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelCreated {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        channel_type: text_to_string(p.get_channel_type().map_err(|e| capnp_err(&e))?)?,
        record_key: text_to_string(p.get_record_key().map_err(|e| capnp_err(&e))?)?,
        category_id: if p.get_has_category_id() {
            Some(category_id_from_capnp(
                p.get_category_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        position: p.get_position(),
        parent_voice_channel_id: if p.get_has_parent_voice_channel_id() {
            Some(channel_id_from_capnp(
                p.get_parent_voice_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_channel_archived(
    p: schema_pkg::channel_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelArchived {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_channel_updated(
    p: schema_pkg::channel_updated_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let forum_tags = if p.get_has_forum_tags() {
        let list = p.get_forum_tags().map_err(|e| capnp_err(&e))?;
        let v: Result<Vec<String>, ProtocolError> = list
            .iter()
            .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
            .collect();
        Some(v?)
    } else {
        None
    };
    let category_id: Option<Option<CategoryId>> = if p.get_has_category_id() {
        if p.get_category_id_present() {
            Some(Some(category_id_from_capnp(
                p.get_category_id().map_err(|e| capnp_err(&e))?,
            )?))
        } else {
            Some(None)
        }
    } else {
        None
    };
    Ok(GovernanceEntry::ChannelUpdated {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        topic: if p.get_has_topic() {
            Some(text_to_string(p.get_topic().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        forum_tags,
        position: if p.get_has_position() {
            Some(p.get_position())
        } else {
            None
        },
        slowmode_seconds: if p.get_has_slowmode_seconds() {
            Some(p.get_slowmode_seconds())
        } else {
            None
        },
        nsfw: if p.get_has_nsfw() {
            Some(p.get_nsfw())
        } else {
            None
        },
        category_id,
        lamport: p.get_lamport(),
    })
}

fn read_role_definition(
    p: schema_pkg::role_definition_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleDefinition {
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        permissions: p.get_permissions(),
        position: p.get_position(),
        color: p.get_color(),
        hoist: p.get_hoist(),
        mentionable: p.get_mentionable(),
        self_assignable: p.get_self_assignable(),
        exclusion_group: if p.get_has_exclusion_group() {
            Some(text_to_string(
                p.get_exclusion_group().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_role_assignment(
    p: schema_pkg::role_assignment_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleAssignment {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_role_unassignment(
    p: schema_pkg::role_unassignment_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleUnassignment {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_ban_entry(
    p: schema_pkg::ban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::BanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_unban_entry(
    p: schema_pkg::unban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::UnbanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_timeout_entry(
    p: schema_pkg::timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::TimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        duration_seconds: p.get_duration_seconds(),
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        started_at: p.get_started_at(),
        lamport: p.get_lamport(),
    })
}

fn read_remove_timeout_entry(
    p: schema_pkg::remove_timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RemoveTimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_community_meta(
    p: schema_pkg::community_meta_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityMeta {
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        description: if p.get_has_description() {
            Some(text_to_string(
                p.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        icon_hash: if p.get_has_icon_hash() {
            Some(text_to_string(
                p.get_icon_hash().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        banner_hash: if p.get_has_banner_hash() {
            Some(text_to_string(
                p.get_banner_hash().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_community_notification_default(
    p: schema_pkg::community_notification_default_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityNotificationDefault {
        level: text_to_string(p.get_level().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_mek_generation_bump(
    p: schema_pkg::m_e_k_generation_bump_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let cascade: Result<Vec<_>, ProtocolError> = p
        .get_cascade_skipped()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(pseudonym_key_from_capnp)
        .collect();
    Ok(GovernanceEntry::MEKGenerationBump {
        generation: p.get_generation(),
        trigger_departed: pseudonym_key_from_capnp(
            p.get_trigger_departed().map_err(|e| capnp_err(&e))?,
        )?,
        cascade_skipped: cascade?,
        lamport: p.get_lamport(),
    })
}

fn read_category_created(
    p: schema_pkg::category_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryCreated {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        position: p.get_position(),
        lamport: p.get_lamport(),
    })
}

fn read_category_archived(
    p: schema_pkg::category_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryArchived {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_permission_overwrite(
    p: schema_pkg::permission_overwrite_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::PermissionOverwrite {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        target_type: text_to_string(p.get_target_type().map_err(|e| capnp_err(&e))?)?,
        target_id: text_to_string(p.get_target_id().map_err(|e| capnp_err(&e))?)?,
        allow: p.get_allow(),
        deny: p.get_deny(),
        lamport: p.get_lamport(),
    })
}

fn read_thread_created(
    p: schema_pkg::thread_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let invited: Result<Vec<_>, ProtocolError> = p
        .get_invited()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(pseudonym_key_from_capnp)
        .collect();
    Ok(GovernanceEntry::ThreadCreated {
        thread_id: thread_id_from_capnp(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        parent_channel_id: channel_id_from_capnp(
            p.get_parent_channel_id().map_err(|e| capnp_err(&e))?,
        )?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        thread_type: text_to_string(p.get_thread_type().map_err(|e| capnp_err(&e))?)?,
        record_key: if p.get_has_record_key() {
            Some(text_to_string(
                p.get_record_key().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        invited: invited?,
        forum_tag: if p.get_has_forum_tag() {
            Some(text_to_string(
                p.get_forum_tag().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        auto_archive_seconds: p.get_auto_archive_seconds(),
        lamport: p.get_lamport(),
    })
}

fn read_thread_archived(
    p: schema_pkg::thread_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ThreadArchived {
        thread_id: thread_id_from_capnp(p.get_thread_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_event_created(
    p: schema_pkg::event_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::EventCreated {
        event_id: event_id_from_capnp(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        description: if p.get_has_description() {
            Some(text_to_string(
                p.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        start_time: p.get_start_time(),
        end_time: if p.get_has_end_time() {
            Some(p.get_end_time())
        } else {
            None
        },
        channel_id: if p.get_has_channel_id() {
            Some(channel_id_from_capnp(
                p.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        cover_image_ref: if p.get_has_cover_image_ref() {
            Some(text_to_string(
                p.get_cover_image_ref().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        creator_pseudonym: if p.get_has_creator_pseudonym() {
            Some(pseudonym_key_from_capnp(
                p.get_creator_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        recurrence: if p.get_has_recurrence() {
            Some(read_recurrence_rule_via_event_capnp(
                p.get_recurrence().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        location: if p.get_has_location() {
            Some(read_event_location_via_event_capnp(
                p.get_location().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        status: if p.get_has_status() {
            Some(event_status_from_capnp(
                p.get_status().map_err(not_in_schema)?,
            ))
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_expression_added(
    p: schema_pkg::expression_added_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let tags: Result<Vec<String>, ProtocolError> = p
        .get_tags()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    // Architecture §18.4 — readers prefer the new `attachment` field.
    // The deprecated inline_data path is silently dropped; receivers
    // who can't reach the AttachmentOffer chunks will see a missing
    // asset (handled by the eager-fetch loop on next governance merge).
    let attachment = if p.get_has_attachment() {
        Some(read_attachment_offer(
            p.get_attachment().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    Ok(GovernanceEntry::ExpressionAdded {
        expression_id: uuid16_from_capnp(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        kind: text_to_string(p.get_kind().map_err(|e| capnp_err(&e))?)?,
        content_hash: text_to_string(p.get_content_hash().map_err(|e| capnp_err(&e))?)?,
        attachment,
        animated: p.get_animated(),
        tags: tags?,
        sound_meta: if p.get_has_sound_meta() {
            Some(read_sound_meta(
                p.get_sound_meta().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        creator_pseudonym: if p.get_has_creator_pseudonym() {
            Some(pseudonym_key_from_capnp(
                p.get_creator_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        created_at: if p.get_has_created_at() {
            Some(p.get_created_at())
        } else {
            None
        },
        available_to_peers: if p.get_has_available_to_peers() {
            Some(p.get_available_to_peers())
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_attachment_offer(
    p: schema_pkg::attachment_offer::Reader<'_>,
) -> Result<rekindle_types::attachment::AttachmentOffer, ProtocolError> {
    let chunk_hashes: Result<Vec<[u8; 32]>, ProtocolError> = p
        .get_chunk_hashes()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|h| {
            let bytes = h.map_err(|e| capnp_err(&e))?;
            bytes.try_into().map_err(|_| {
                ProtocolError::Deserialization("attachment chunk hash not 32 bytes".into())
            })
        })
        .collect();
    let merkle_root: [u8; 32] = p
        .get_merkle_root()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| {
            ProtocolError::Deserialization("attachment merkle_root not 32 bytes".into())
        })?;
    Ok(rekindle_types::attachment::AttachmentOffer {
        attachment_id: uuid16_from_capnp(p.get_attachment_id().map_err(|e| capnp_err(&e))?)?,
        filename: text_to_string(p.get_filename().map_err(|e| capnp_err(&e))?)?,
        mime_type: text_to_string(p.get_mime_type().map_err(|e| capnp_err(&e))?)?,
        total_size: p.get_total_size(),
        chunk_count: p.get_chunk_count(),
        chunk_size: p.get_chunk_size(),
        merkle_root,
        chunk_hashes: chunk_hashes?,
        wrapped_fek: p.get_wrapped_fek().map_err(|e| capnp_err(&e))?.to_vec(),
        fek_mek_generation: p.get_fek_mek_generation(),
    })
}

fn read_expression_removed(
    p: schema_pkg::expression_removed_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ExpressionRemoved {
        expression_id: uuid16_from_capnp(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_event_archived(
    p: schema_pkg::event_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::EventArchived {
        event_id: event_id_from_capnp(p.get_event_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_onboarding_config(
    p: schema_pkg::onboarding_config_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let default_channels: Result<Vec<ChannelId>, ProtocolError> = p
        .get_default_channels()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(channel_id_from_capnp)
        .collect();
    let questions: Result<Vec<OnboardingQuestion>, ProtocolError> = p
        .get_questions()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_question)
        .collect();
    let guide_steps: Result<Vec<GuideStep>, ProtocolError> = p
        .get_guide_steps()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_guide_step)
        .collect();
    Ok(GovernanceEntry::OnboardingConfig {
        enabled: p.get_enabled(),
        mode: text_to_string(p.get_mode().map_err(|e| capnp_err(&e))?)?,
        default_channels: default_channels?,
        questions: questions?,
        welcome_message: if p.get_has_welcome_message() {
            Some(text_to_string(
                p.get_welcome_message().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        guide_steps: guide_steps?,
        lamport: p.get_lamport(),
    })
}

fn read_welcome_screen(
    p: schema_pkg::welcome_screen_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let channels: Result<Vec<WelcomeChannel>, ProtocolError> = p
        .get_channels()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_welcome_channel)
        .collect();
    Ok(GovernanceEntry::WelcomeScreen {
        description: text_to_string(p.get_description().map_err(|e| capnp_err(&e))?)?,
        channels: channels?,
        lamport: p.get_lamport(),
    })
}

fn read_admin_delete(
    p: schema_pkg::admin_delete_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AdminDelete {
        message_id: uuid16_from_capnp(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_channel_segment_linked(
    p: schema_pkg::channel_segment_linked_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ChannelSegmentLinked {
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        segment_index: p.get_segment_index(),
        record_key: text_to_string(p.get_record_key().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_segment_added(
    p: schema_pkg::segment_added_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::SegmentAdded {
        segment_index: p.get_segment_index(),
        registry_key: text_to_string(p.get_registry_key().map_err(|e| capnp_err(&e))?)?,
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
        slot_range_start: p.get_slot_range_start(),
        slot_range_end: p.get_slot_range_end(),
        lamport: p.get_lamport(),
    })
}

fn read_auto_mod_rule(
    p: schema_pkg::auto_mod_rule_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AutoModRule {
        rule_id: uuid16_from_capnp(p.get_rule_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        enabled: p.get_enabled(),
        trigger_json: text_to_string(p.get_trigger_json().map_err(|e| capnp_err(&e))?)?,
        action: text_to_string(p.get_action().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_role_archived(
    p: schema_pkg::role_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleArchived {
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_category_updated(
    p: schema_pkg::category_updated_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CategoryUpdated {
        category_id: category_id_from_capnp(p.get_category_id().map_err(|e| capnp_err(&e))?)?,
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        position: if p.get_has_position() {
            Some(p.get_position())
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

fn read_invite_created(
    p: schema_pkg::invite_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::InviteCreated {
        invite_id: uuid16_from_capnp(p.get_invite_id().map_err(|e| capnp_err(&e))?)?,
        code_hash: text_to_string(p.get_code_hash().map_err(|e| capnp_err(&e))?)?,
        max_uses: p.get_max_uses(),
        expires_at: if p.get_has_expires_at() {
            Some(p.get_expires_at())
        } else {
            None
        },
        encrypted_secrets: text_to_string(p.get_encrypted_secrets().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_invite_revoked(
    p: schema_pkg::invite_revoked_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::InviteRevoked {
        invite_id: uuid16_from_capnp(p.get_invite_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

fn read_attachment_pinned(
    p: schema_pkg::attachment_pinned_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AttachmentPinned {
        attachment_id: uuid16_from_capnp(p.get_attachment_id().map_err(|e| capnp_err(&e))?)?,
        pinned: p.get_pinned(),
        lamport: p.get_lamport(),
    })
}

fn read_community_policy(
    p: schema_pkg::community_policy_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityPolicy {
        policy_text: if p.get_has_policy_text() {
            Some(text_to_string(
                p.get_policy_text().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        max_joins_per_interval: p.get_max_joins_per_interval(),
        join_interval_seconds: p.get_join_interval_seconds(),
        lamport: p.get_lamport(),
    })
}

// ── Sub-type helpers ─────────────────────────────────────────────────

fn event_status_to_capnp(
    s: rekindle_types::event::EventStatus,
) -> community_event_capnp::EventStatus {
    use community_event_capnp::EventStatus as Cap;
    use rekindle_types::event::EventStatus;
    match s {
        EventStatus::Scheduled => Cap::Scheduled,
        EventStatus::Active => Cap::Active,
        EventStatus::Completed => Cap::Completed,
        EventStatus::Cancelled => Cap::Cancelled,
    }
}

fn event_status_from_capnp(
    s: community_event_capnp::EventStatus,
) -> rekindle_types::event::EventStatus {
    use community_event_capnp::EventStatus as Cap;
    use rekindle_types::event::EventStatus;
    match s {
        Cap::Scheduled => EventStatus::Scheduled,
        Cap::Active => EventStatus::Active,
        Cap::Completed => EventStatus::Completed,
        Cap::Cancelled => EventStatus::Cancelled,
    }
}

fn write_sound_meta(mut b: schema_pkg::soundboard_meta::Builder<'_>, s: &SoundboardMeta) {
    b.set_duration_seconds(s.duration_seconds);
    b.set_volume(s.volume);
    b.set_has_emoji(s.emoji.is_some());
    if let Some(ref e) = s.emoji {
        b.set_emoji(e);
    }
}

fn read_sound_meta(
    r: schema_pkg::soundboard_meta::Reader<'_>,
) -> Result<SoundboardMeta, ProtocolError> {
    Ok(SoundboardMeta {
        duration_seconds: r.get_duration_seconds(),
        volume: r.get_volume(),
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

fn write_onboarding_question(
    mut b: schema_pkg::onboarding_question::Builder<'_>,
    q: &OnboardingQuestion,
) {
    b.set_question_id(&q.question_id);
    b.set_title(&q.title);
    b.set_has_description(q.description.is_some());
    if let Some(ref d) = q.description {
        b.set_description(d);
    }
    b.set_required(q.required);
    b.set_single_select(q.single_select);
    let mut opts = b.reborrow().init_options(len_u32(q.options.len()));
    for (i, o) in q.options.iter().enumerate() {
        write_onboarding_option(opts.reborrow().get(len_u32(i)), o);
    }
}

fn read_onboarding_question(
    r: schema_pkg::onboarding_question::Reader<'_>,
) -> Result<OnboardingQuestion, ProtocolError> {
    let options: Result<Vec<OnboardingOption>, ProtocolError> = r
        .get_options()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_onboarding_option)
        .collect();
    Ok(OnboardingQuestion {
        question_id: text_to_string(r.get_question_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: if r.get_has_description() {
            Some(text_to_string(
                r.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        required: r.get_required(),
        single_select: r.get_single_select(),
        options: options?,
    })
}

fn write_onboarding_option(
    mut b: schema_pkg::onboarding_option::Builder<'_>,
    o: &OnboardingOption,
) {
    b.set_option_id(&o.option_id);
    b.set_title(&o.title);
    b.set_has_description(o.description.is_some());
    if let Some(ref d) = o.description {
        b.set_description(d);
    }
    b.set_has_emoji(o.emoji.is_some());
    if let Some(ref e) = o.emoji {
        b.set_emoji(e);
    }
    let mut roles = b
        .reborrow()
        .init_roles_to_assign(len_u32(o.roles_to_assign.len()));
    for (i, r) in o.roles_to_assign.iter().enumerate() {
        uuid16_to_capnp(roles.reborrow().get(len_u32(i)), &r.0);
    }
    let mut chans = b
        .reborrow()
        .init_channels_to_show(len_u32(o.channels_to_show.len()));
    for (i, c) in o.channels_to_show.iter().enumerate() {
        uuid16_to_capnp(chans.reborrow().get(len_u32(i)), &c.0);
    }
}

fn read_onboarding_option(
    r: schema_pkg::onboarding_option::Reader<'_>,
) -> Result<OnboardingOption, ProtocolError> {
    let roles_to_assign: Result<Vec<RoleId>, ProtocolError> = r
        .get_roles_to_assign()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(role_id_from_capnp)
        .collect();
    let channels_to_show: Result<Vec<ChannelId>, ProtocolError> = r
        .get_channels_to_show()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(channel_id_from_capnp)
        .collect();
    Ok(OnboardingOption {
        option_id: text_to_string(r.get_option_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: if r.get_has_description() {
            Some(text_to_string(
                r.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        roles_to_assign: roles_to_assign?,
        channels_to_show: channels_to_show?,
    })
}

fn write_guide_step(mut b: schema_pkg::guide_step::Builder<'_>, s: &GuideStep) {
    b.set_title(&s.title);
    b.set_description(&s.description);
    b.set_has_channel_id(s.channel_id.is_some());
    if let Some(ref c) = s.channel_id {
        uuid16_to_capnp(b.reborrow().init_channel_id(), &c.0);
    }
    b.set_has_emoji(s.emoji.is_some());
    if let Some(ref e) = s.emoji {
        b.set_emoji(e);
    }
}

fn read_guide_step(r: schema_pkg::guide_step::Reader<'_>) -> Result<GuideStep, ProtocolError> {
    Ok(GuideStep {
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: text_to_string(r.get_description().map_err(|e| capnp_err(&e))?)?,
        channel_id: if r.get_has_channel_id() {
            Some(channel_id_from_capnp(
                r.get_channel_id().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

fn write_welcome_channel(mut b: schema_pkg::welcome_channel::Builder<'_>, w: &WelcomeChannel) {
    uuid16_to_capnp(b.reborrow().init_channel_id(), &w.channel_id.0);
    b.set_description(&w.description);
    b.set_has_emoji(w.emoji.is_some());
    if let Some(ref e) = w.emoji {
        b.set_emoji(e);
    }
}

fn read_welcome_channel(
    r: schema_pkg::welcome_channel::Reader<'_>,
) -> Result<WelcomeChannel, ProtocolError> {
    Ok(WelcomeChannel {
        channel_id: channel_id_from_capnp(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        description: text_to_string(r.get_description().map_err(|e| capnp_err(&e))?)?,
        emoji: if r.get_has_emoji() {
            Some(text_to_string(r.get_emoji().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}
