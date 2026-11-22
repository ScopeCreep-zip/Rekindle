//! A community window's reads on the Host (plan E3.1 D1b), both modes:
//! every joined community with its channels, categories and roles, and one
//! community's member list over the stored roster and the online overlay.

use std::collections::HashMap;

use rekindle_types::dto::community::{
    CommunityCategoryView, CommunityChannelView, CommunityMemberView, CommunityRoleView,
    CommunityView, MemberGroup, TargetModeration,
};
use rekindle_types::dto::member_groups::{group_members, GroupRole};
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionLocation;

use rekindle_skin::role_tag::SkinGrounds;

use super::HostGovernance;
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
        (c.my_pseudonym_key.as_deref())
            .and_then(|hex| hex::decode(hex).ok()?.try_into().ok())
            .map(PseudonymKey),
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

/// Whether we hold a creator seat in `c`'s merged governance (plan E3.2
/// S7s, `holds_creator_seat`); false before the first merge.
fn we_are_creator(c: &CommunityState) -> bool {
    let me = (c.my_pseudonym_key.as_deref())
        .and_then(|hex| hex::decode(hex).ok()?.try_into().ok())
        .map(PseudonymKey);
    match (c.governance_state.as_ref(), me) {
        (Some(gov), Some(me)) => rekindle_governance::permissions::holds_creator_seat(&me, gov),
        _ => false,
    }
}

fn community_view(c: &CommunityState, now_secs: u64, shown: Option<&SkinGrounds>) -> CommunityView {
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
            .map(|channel| CommunityChannelView {
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
        roles: c.roles.iter().map(|r| role_view(r, shown)).collect(),
        my_pseudonym_key: c.my_pseudonym_key.clone(),
        mek_generation: c.mek_generation,
        member_registry_key: c.member_registry_key.clone(),
        anchor: c.anchor.clone(),
        onboarding_complete: c.onboarding_complete,
        my_bio: c.my_bio.clone(),
        my_pronouns: c.my_pronouns.clone(),
        my_theme_color: c.my_theme_color,
        my_badges: c.my_badges.clone(),
        removed: None,
        op_publications: std::collections::BTreeMap::new(),
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
        (removal.by.as_ref()).and_then(|by| communities.removal_name_of(community, by))
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
    /// Status, location and last-active per online pseudonym.
    online: HashMap<String, (String, Option<SessionLocation>, u64)>,
    /// Peers' profiles, ours from our own fields.
    profiles: HashMap<String, CommunityMemberView>,
    my_location: Option<SessionLocation>,
}

fn overlay(c: &CommunityState) -> RosterOverlay {
    let online = c
        .gossip
        .as_ref()
        .map(|g| {
            g.online_members
                .iter()
                .map(|(pk, m)| {
                    (
                        pk.clone(),
                        (m.status.clone(), m.location.clone(), m.last_active),
                    )
                })
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
        online,
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
        can_moderate_target: TargetModeration::default(),
        outranks: false,
    }
}

/// What we may do to `target` by the merged governance `gov` (plan E3.2
/// S9): each moderation action's validator verdict, and whether we outrank
/// them; nothing to ourselves, nothing before we hold a pseudonym.
fn moderation_of(
    gov: &rekindle_governance::state::GovernanceState,
    me: Option<&PseudonymKey>,
    target: &PseudonymKey,
) -> (TargetModeration, bool) {
    use rekindle_governance::validate::target::{may_moderate, outranks, TargetAction};
    let Some(me) = me.filter(|me| *me != target) else {
        return (TargetModeration::default(), false);
    };
    (
        TargetModeration {
            kick: may_moderate(me, target, TargetAction::Kick, gov),
            ban: may_moderate(me, target, TargetAction::Ban, gov),
            timeout: may_moderate(me, target, TargetAction::Timeout, gov),
        },
        outranks(me, target, gov),
    )
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
            .map(|c| community_view(c, now_secs, shown.as_deref()))
            .collect();
        for view in &mut views {
            view.removed = removal_display(&self.ctx.communities, &view.id);
        }
        if let Some(ops) = self.ctx.outbox.governance_ops() {
            for view in &mut views {
                match ops.publications(&view.id).await {
                    Ok(states) => view.op_publications = states,
                    Err(error) => tracing::warn!(%error, "queued governance ops not read"),
                }
            }
        }
        views.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        views
    }

    /// `community`'s members by display name: the stored roster with the
    /// online overlay's status and location; ours from our own presence.
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
        let my_status = self.ctx.own_presence.status().as_wire_str().to_owned();
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
                let online = overlay.online.get(&row.pseudonym_key);
                let (status, location, last_active) = if is_me {
                    (my_status.clone(), overlay.my_location.clone(), now)
                } else {
                    online.map_or_else(
                        || ("offline".to_owned(), None, 0),
                        |(s, l, a)| (s.clone(), l.clone(), *a),
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
                let card = (self.ctx.communities).member_card_name(community, &target);
                let (can_moderate_target, outranks) =
                    moderation_of(&governance, me.as_ref(), &target);
                CommunityMemberView {
                    display_role: display_role_name(&role_ids, &overlay.roles),
                    display_name: card.or(row.display_name).unwrap_or_default(),
                    role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
                    status,
                    timeout_until: timeout_until(&row.pseudonym_key),
                    location,
                    last_active,
                    pseudonym_key: row.pseudonym_key,
                    can_moderate_target,
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
