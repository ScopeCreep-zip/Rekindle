//! A community window's reads on the Host (plan E3.1 D1b), both modes:
//! every joined community with its channels, categories and roles, and one
//! community's member list over the stored roster and the online overlay.

use std::collections::HashMap;

use rekindle_types::dto::community::{
    CommunityCategoryView, CommunityChannelView, CommunityMemberView, CommunityRoleView,
    CommunityView, MemberGroup, MemberModeration,
};
use rekindle_types::dto::member_groups::{group_members, GroupRole};
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::SessionLocation;

use rekindle_skin::role_tag::SkinGrounds;

use super::capability_words::joined_pseudonym;
use super::HostGovernance;
use crate::host::adapters::presence::view as beat_view;
use crate::host::community::{display_role_name, CommunityState, RoleDefinition};

fn role_view(role: &RoleDefinition, shown: Option<&SkinGrounds>) -> CommunityRoleView {
    CommunityRoleView {
        id: role.id.to_hex(),
        name: role.name.clone(),
        color: role.color,
        tag_colours: rekindle_skin::role_tag::colours(role.color, shown),
        permissions: role.permissions.to_string(),
        position: role.position,
        hoist: role.hoist,
        mentionable: role.mentionable,
        self_assignable: role.self_assignable,
        exclusion_group: role.exclusion_group.clone(),
    }
}

/// Our base and per-channel effective permissions in `c` at `now_secs`,
/// from its merged governance; none before the first merge.
fn my_permissions(
    c: &CommunityState,
    now_secs: u64,
) -> (String, std::collections::BTreeMap<String, String>) {
    use rekindle_governance::permissions::compute_permissions;
    let (Some(gov), Some(me)) = (
        c.governance_state.as_ref(),
        c.my_pseudonym_key.as_deref().and_then(joined_pseudonym),
    ) else {
        return ("0".into(), std::collections::BTreeMap::new());
    };
    let channels = gov
        .channels
        .keys()
        .map(|id| {
            let perms = compute_permissions(&me, Some(id), gov, now_secs);
            (hex::encode(id.0), perms.to_string())
        })
        .collect();
    (
        compute_permissions(&me, None, gov, now_secs).to_string(),
        channels,
    )
}

/// The window controls we may not use in `c` at `now_secs`, each with
/// the Host's reason (plan E3.2 S9); before the first merge every one,
/// for no governance is merged yet.
fn my_denied(
    c: &CommunityState,
    now_secs: u64,
) -> rekindle_types::dto::community_capability::DeniedCapabilities {
    let me = c.my_pseudonym_key.as_deref().and_then(joined_pseudonym);
    match (c.governance_state.as_ref(), me) {
        (Some(gov), Some(me)) => super::capability_words::denied_of(&me, gov, now_secs),
        _ => (rekindle_types::dto::community_capability::CommunityCapability::ALL)
            .into_iter()
            .map(|cap| (cap, super::capability_words::NOT_LOADED.to_owned()))
            .collect(),
    }
}

/// Whether we hold a creator seat in `c`'s merged governance (plan E3.2
/// S7s, `holds_creator_seat`); false before the first merge.
fn we_are_creator(c: &CommunityState) -> bool {
    let me = c.my_pseudonym_key.as_deref().and_then(joined_pseudonym);
    match (c.governance_state.as_ref(), me) {
        (Some(gov), Some(me)) => rekindle_governance::permissions::holds_creator_seat(&me, gov),
        _ => false,
    }
}

/// Our card in `c` as the window renders it (plan E3.4 S6): the name it
/// shows (never empty, never the identity's), the name as set, and the
/// custom status.
fn own_card_view(
    c: &CommunityState,
    card: Option<rekindle_governance_runtime::CardText>,
) -> (String, Option<String>, Option<String>) {
    let card = card.unwrap_or_default();
    let shown = c
        .my_pseudonym_key
        .as_deref()
        .map(PseudonymKey::from_hex_lossy)
        .map(|me| super::community_profile::shown_name(card.display_name.clone(), &me))
        .unwrap_or_default();
    (shown, card.display_name, card.custom_status)
}

/// Why we cannot read `channel` (hex) of `c`, in the Host's words; none
/// when our effective permissions there include `VIEW_CHANNELS`, or before
/// the first merge.
pub(crate) fn channel_lock(c: &CommunityState, channel: &str, now_secs: u64) -> Option<String> {
    use rekindle_governance::permissions::{compute_permissions, is_restricted};
    use rekindle_types::permissions::VIEW_CHANNELS;
    let gov = c.governance_state.as_ref()?;
    let me = c.my_pseudonym_key.as_deref().and_then(joined_pseudonym)?;
    let id = ChannelId::from_hex(channel)?;
    if compute_permissions(&me, Some(&id), gov, now_secs) & VIEW_CHANNELS != 0 {
        return None;
    }
    Some(if is_restricted(gov, &id) {
        "This channel is limited to roles you don't hold.".into()
    } else {
        "You don't have permission to view this channel.".into()
    })
}

/// The controls of `channel` (hex) of `c` we may not use, with the Host's
/// reasons (plan E3.2 S9); before the first merge every one.
fn channel_denied(
    c: &CommunityState,
    channel: &str,
    now_secs: u64,
) -> rekindle_types::dto::community_capability::ChannelDeniedCapabilities {
    use rekindle_types::dto::community_capability::ChannelCapability;
    let me = c.my_pseudonym_key.as_deref().and_then(joined_pseudonym);
    let id = ChannelId::from_hex(channel);
    match (c.governance_state.as_ref(), me, id) {
        (Some(gov), Some(me), Some(id)) => {
            super::capability_words::channel_denied_of(&me, &id, gov, now_secs)
        }
        _ => (ChannelCapability::ALL.into_iter())
            .map(|cap| (cap, super::capability_words::NOT_LOADED.to_owned()))
            .collect(),
    }
}

fn community_view(
    c: &CommunityState,
    own_card: (String, Option<String>, Option<String>),
    now_secs: u64,
    shown: Option<&SkinGrounds>,
) -> CommunityView {
    let (my_display_name, my_card_name, my_custom_status) = own_card;
    let (my_permissions, my_channel_permissions) = my_permissions(c, now_secs);
    CommunityView {
        id: c.id.clone(),
        name: c.name.clone(),
        description: c.description.clone(),
        icon_hash: c.icon_hash.clone(),
        banner_hash: c.banner_hash.clone(),
        channels: c
            .channels
            .iter()
            .map(|channel| (channel, channel_lock(c, &channel.id, now_secs)))
            .map(|(channel, lock)| CommunityChannelView {
                id: channel.id.clone(),
                name: channel.name.clone(),
                channel_type: channel.channel_type.to_string(),
                category_id: channel.category_id.clone(),
                topic: channel.topic.clone(),
                forum_tags: channel.forum_tags.clone(),
                stage_speakers: channel.stage_speakers.clone(),
                stage_moderator: channel.stage_moderator.clone(),
                slowmode_seconds: channel.slowmode_seconds,
                notification_level: channel.notification_level.clone(),
                notification_sound_ref: channel.notification_sound_ref.clone(),
                locked: lock.is_some(),
                locked_reason: lock.unwrap_or_default(),
                denied: channel_denied(c, &channel.id, now_secs),
            })
            .collect(),
        categories: c
            .categories
            .iter()
            .map(|category| CommunityCategoryView {
                id: category.id.clone(),
                name: category.name.clone(),
                sort_order: category.sort_order,
            })
            .collect(),
        my_role: Some(display_role_name(&c.my_role_ids, &c.roles)),
        my_role_ids: c.my_role_ids.iter().map(|id| id.to_hex()).collect(),
        my_permissions,
        my_channel_permissions,
        we_are_creator: we_are_creator(c),
        denied: my_denied(c, now_secs),
        roles: c.roles.iter().map(|r| role_view(r, shown)).collect(),
        my_pseudonym_key: c.my_pseudonym_key.clone(),
        // The same state `CommunityStore::mek_generation` serves to info.
        mek_generation: c.mek_generation,
        member_registry_key: c.member_registry_key.clone(),
        anchor: c.anchor.clone(),
        onboarding_complete: c.onboarding_complete,
        my_bio: c.my_bio.clone(),
        my_pronouns: c.my_pronouns.clone(),
        my_theme_color: c.my_theme_color,
        my_badges: c.my_badges.clone(),
        my_avatar_ref: c.my_avatar_ref.clone(),
        my_banner_ref: c.my_banner_ref.clone(),
        my_display_name,
        my_card_name,
        my_custom_status,
        removed: None,
        op_publications: std::collections::BTreeMap::new(),
        device_status: rekindle_types::dto::community::DeviceStatus::Enrolled,
        catch_up: None,
        key_status: rekindle_types::dto::community::KeyStatus::default(),
    }
}

/// Our removal from `community`, as every community read shows it (plan
/// E3.2 S9 own removal, R3-5, R5-2): who removed us, by pseudonym and the
/// name kept with the removal (else its card or profile now), and why.
pub(crate) fn removal_display(
    communities: &crate::host::community::store::CommunityStore,
    community: &str,
) -> Option<rekindle_types::display::CommunityRemovalDisplay> {
    let removal = communities.own_removal(community)?;
    let by_name = removal.by_name.clone().or_else(|| {
        (removal.by.as_ref()).map(|by| {
            super::community_profile::shown_name(communities.removal_name_of(community, by), by)
        })
    });
    Some(rekindle_types::display::CommunityRemovalDisplay {
        banned: removal.banned,
        by: removal
            .by
            .as_ref()
            .map(rekindle_types::id::PseudonymKey::to_hex),
        by_name,
        reason: removal.reason,
    })
}

/// What the store holds about a community's members, read in one lock.
#[derive(Default)]
struct RosterOverlay {
    me: Option<String>,
    roles: Vec<RoleDefinition>,
    /// Last-active per pseudonym from the legacy gossip mesh.
    last_active: HashMap<String, u64>,
    /// Peers' profiles, ours from our own fields.
    profiles: HashMap<String, CommunityMemberView>,
    my_location: Option<SessionLocation>,
}

fn overlay(c: &CommunityState) -> RosterOverlay {
    let last_active = c
        .gossip
        .as_ref()
        .map(|g| {
            g.online_members
                .iter()
                .map(|(pk, m)| (pk.clone(), m.last_active))
                .collect()
        })
        .unwrap_or_default();
    let mut profiles: HashMap<String, CommunityMemberView> = c
        .member_profiles
        .iter()
        .map(|(pk, p)| {
            (
                pk.clone(),
                profile_view(
                    p.bio.clone(),
                    p.pronouns.clone(),
                    p.theme_color,
                    p.badges.clone(),
                    p.avatar_ref.clone(),
                    p.banner_ref.clone(),
                ),
            )
        })
        .collect();
    if let Some(me) = &c.my_pseudonym_key {
        profiles.insert(
            me.clone(),
            profile_view(
                c.my_bio.clone(),
                c.my_pronouns.clone(),
                c.my_theme_color,
                c.my_badges.clone(),
                c.my_avatar_ref.clone(),
                c.my_banner_ref.clone(),
            ),
        );
    }
    RosterOverlay {
        me: c.my_pseudonym_key.clone(),
        roles: c.roles.clone(),
        last_active,
        profiles,
        my_location: c.my_session_location.clone(),
    }
}

/// A member view carrying only the profile fields.
fn profile_view(
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
) -> CommunityMemberView {
    CommunityMemberView {
        pseudonym_key: String::new(),
        display_name: String::new(),
        fingerprint: String::new(),
        custom_status: None,
        role_ids: Vec::new(),
        display_role: String::new(),
        status: String::new(),
        timeout_until: None,
        bio,
        pronouns,
        theme_color,
        badges,
        avatar_ref,
        banner_ref,
        location: None,
        last_active: 0,
        group: MemberGroup::Offline,
        moderation: MemberModeration::refused(super::capability_words::JOIN_UNFINISHED),
        outranks: false,
    }
}

/// What we may do to `target` by the merged governance `gov` (plan E3.2
/// S9): each moderation action's validator verdict, and whether we outrank
/// them; nothing to ourselves, nothing before we hold a pseudonym, each
/// refusal worded by the Host (`capability_words`).
fn moderation_of(
    gov: &rekindle_governance::state::GovernanceState,
    me: Option<&PseudonymKey>,
    target: &PseudonymKey,
) -> (MemberModeration, bool) {
    use super::capability_words::{moderation_words, JOIN_UNFINISHED, YOURSELF};
    use rekindle_governance::validate::target::{
        moderation_refusal, outranks, role_menu_refusal, TargetAction,
    };
    let Some(me) = me else {
        return (MemberModeration::refused(JOIN_UNFINISHED), false);
    };
    if me == target {
        return (MemberModeration::refused(YOURSELF), false);
    }
    let verdict = |action| moderation_refusal(me, target, action, gov).map(moderation_words);
    (
        MemberModeration {
            kick: verdict(TargetAction::Kick),
            ban: verdict(TargetAction::Ban),
            timeout: verdict(TargetAction::Timeout),
            manage_roles: role_menu_refusal(me, target, gov).map(moderation_words),
            mute: verdict(TargetAction::Mute),
            deafen: verdict(TargetAction::Deafen),
        },
        outranks(me, target, gov),
    )
}

/// Where each of our ops of `community` not yet landed stands, by op hash
/// (hex; the outbox's `governance_op` rows): the one source the window's
/// view and the daemon's `community info` both read, so every frontend
/// shows the same state. Empty while no outbox runs (locked).
pub(crate) async fn op_publications(
    ctx: &crate::daemon::dispatch::DaemonContext,
    community: &str,
) -> std::collections::BTreeMap<String, rekindle_types::subscription_events::OpPublicationState> {
    let Some(ops) = ctx.outbox.governance_ops() else {
        return std::collections::BTreeMap::new();
    };
    match ops.publications(community).await {
        Ok(states) => states,
        Err(error) => {
            tracing::warn!(%error, "queued governance ops not read");
            std::collections::BTreeMap::new()
        }
    }
}

impl HostGovernance {
    /// Every joined community, by name then id, with where each of our
    /// ops not yet landed stands (the outbox's `governance_op` rows).
    pub(crate) async fn community_views(&self) -> Vec<CommunityView> {
        let now_secs = self.ctx.clock.now_secs();
        let shown = self.ctx.skin.grounds.get();
        let mut views: Vec<CommunityView> = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            // A community we are leaving is listed nowhere (plan E3.2
            // S7d-keys.8).
            .filter(|c| !c.departing)
            .map(|c| {
                let card = (c.my_pseudonym_key.as_deref())
                    .map(PseudonymKey::from_hex_lossy)
                    .and_then(|me| self.ctx.communities.member_card(&c.id, &me));
                community_view(c, own_card_view(c, card), now_secs, shown.as_deref())
            })
            .collect();
        for view in &mut views {
            view.removed = removal_display(&self.ctx.communities, &view.id);
            view.catch_up = self.ctx.communities.unlock_barriers().view(&view.id);
            if view.removed.is_none() {
                view.device_status = self.device_status(&view.id);
            }
            let held = self.held_community_generation(&view.id);
            if let Some(c) = self.ctx.communities.map().read().get(&view.id) {
                view.key_status = super::key_status::key_status(c, held);
            }
        }
        for view in &mut views {
            view.op_publications = op_publications(&self.ctx, &view.id).await;
        }
        views.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        views
    }

    /// `community`'s members by display name: the stored roster, each
    /// other member's status as its beats show it (plan E3.4 S3; a founder
    /// never announces a join, so the gossip mesh never knew her online),
    /// with its verified beat location and gossip last-active; ours from our
    /// own presence, by the rule the online count uses
    /// (`beat_view::own_shown`).
    ///
    /// # Errors
    /// No identity is unlocked or the roster could not be read.
    pub(crate) async fn community_members(
        &self,
        community: &str,
    ) -> Result<Vec<CommunityMemberView>, String> {
        let overlay = self
            .ctx
            .communities
            .map()
            .read()
            .get(community)
            .map(overlay)
            .unwrap_or_default();
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let cid = community.to_owned();
        let rows = db
            .call(move |conn| rekindle_db::repo::members::roster(conn, &owner, &cid))
            .await
            .map_err(|e| format!("community members: {e}"))?;
        let my_status = beat_view::own_shown(&self.ctx).as_wire_str().to_owned();
        let now = self.ctx.clock.now_secs();
        // Bans, removals and timeouts are the merged governance's (plan
        // E3.1 F2, E3.2): a banned or removed member leaves the list, as the
        // merge decides it on every reader (the roster row is deleted only
        // where the departure was seen first-hand), and a timeout ends at
        // its time.
        // Only the members the merge counts are listed (finding I1): a row
        // stored from a gossip announce, a registry read or a member record
        // of a key the merge never admitted (a joiner past the invite's
        // `max_uses`) never shows as a member.
        let governance = self.governance_state_impl(community).unwrap_or_default();
        let listed = super::projection::listed(&governance);
        // A member's roles are the merged role assignments (plan E3.2, D15),
        // as our own are (`projection::project_my_roles`), built once for
        // the list (review RV7 L4); no row caches them.
        let role_ids_of = super::projection::member_role_ids(&governance);
        let me = overlay.me.as_deref().map(PseudonymKey::from_hex_lossy);
        let timeout_until = |pseudonym: &str| {
            governance
                .timeouts
                .iter()
                .find(|(p, _)| hex::encode(p.0) == pseudonym)
                .map(|(_, t)| t.started_at.saturating_add(t.duration_seconds))
                .filter(|until| *until > now)
        };
        let members = rows
            .into_iter()
            .filter(|row| listed.contains(&row.pseudonym_key))
            .map(|row| {
                let is_me = overlay.me.as_deref() == Some(row.pseudonym_key.as_str());
                let (status, location, last_active) = if is_me {
                    (my_status.clone(), overlay.my_location.clone(), now)
                } else {
                    let (shown, location) = beat_view::shown_location(
                        &self.ctx,
                        community,
                        &row.pseudonym_key,
                        governance
                            .members
                            .get(&PseudonymKey::from_hex_lossy(&row.pseudonym_key))
                            .map(|member| member.member_record.as_str()),
                    );
                    let last_active = overlay
                        .last_active
                        .get(&row.pseudonym_key)
                        .copied()
                        .unwrap_or_default();
                    (
                        beat_view::shown_status(shown).to_owned(),
                        location,
                        last_active,
                    )
                };
                let profile = overlay
                    .profiles
                    .get(&row.pseudonym_key)
                    .cloned()
                    .unwrap_or_else(|| profile_view(None, None, None, Vec::new(), None, None));
                let role_ids = role_ids_of
                    .get(&row.pseudonym_key)
                    .cloned()
                    .unwrap_or_default();
                // The verified card's name first (plan E3.2 F9-2), so a
                // re-read never undoes a `MemberCardChanged`.
                let target = PseudonymKey::from_hex_lossy(&row.pseudonym_key);
                let card = (self.ctx.communities)
                    .member_card(community, &target)
                    .unwrap_or_default();
                let (moderation, outranks) = moderation_of(&governance, me.as_ref(), &target);
                CommunityMemberView {
                    display_role: display_role_name(&role_ids, &overlay.roles),
                    // Never empty (plan E3.4 S6): the card's name, else
                    // the short pseudonym fingerprint.
                    display_name: super::community_profile::shown_name(
                        card.display_name.or(row.display_name),
                        &target,
                    ),
                    fingerprint: rekindle_crypto::identity::short_key_fingerprint(&target.0),
                    custom_status: card.custom_status,
                    role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
                    status,
                    timeout_until: timeout_until(&row.pseudonym_key),
                    location,
                    last_active,
                    pseudonym_key: row.pseudonym_key,
                    moderation,
                    outranks,
                    ..profile
                }
            })
            .collect();
        Ok(group_members(
            members,
            overlay.roles.iter().map(|r| GroupRole {
                id: r.id,
                name: &r.name,
                position: r.position,
                hoist: r.hoist,
            }),
        ))
    }
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
