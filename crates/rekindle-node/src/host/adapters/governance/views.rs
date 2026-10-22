//! A community window's reads on the Host (plan E3.1 D1b), both modes:
//! every joined community with its channels, categories and roles, and one
//! community's member list over the stored roster and the online overlay.

use std::collections::HashMap;

use rekindle_types::dto::community::{
    CommunityCategoryView, CommunityChannelView, CommunityMemberView, CommunityRoleView,
    CommunityView,
};
use rekindle_types::presence::SessionLocation;

use super::HostGovernance;
use crate::host::community::{display_role_name, CommunityState, RoleDefinition};

fn role_view(role: &RoleDefinition) -> CommunityRoleView {
    CommunityRoleView {
        id: role.id,
        name: role.name.clone(),
        color: role.color,
        permissions: role.permissions.to_string(),
        position: role.position,
        hoist: role.hoist,
        mentionable: role.mentionable,
        self_assignable: role.self_assignable,
        exclusion_group: role.exclusion_group.clone(),
    }
}

fn community_view(c: &CommunityState) -> CommunityView {
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
                unread_count: channel.unread_count,
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
        my_role_ids: c.my_role_ids.clone(),
        roles: c.roles.iter().map(role_view).collect(),
        my_pseudonym_key: c.my_pseudonym_key.clone(),
        mek_generation: c.mek_generation,
        member_registry_key: c.member_registry_key.clone(),
        governance_key: c.governance_key.clone(),
        onboarding_complete: c.onboarding_complete,
        my_bio: c.my_bio.clone(),
        my_pronouns: c.my_pronouns.clone(),
        my_theme_color: c.my_theme_color,
        my_badges: c.my_badges.clone(),
    }
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
    }
}

impl HostGovernance {
    /// Every joined community, by name then id.
    pub(crate) fn community_views(&self) -> Vec<CommunityView> {
        let mut views: Vec<CommunityView> = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            .map(community_view)
            .collect();
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
        // Bans and timeouts are the merged governance's (plan E3.1 F2): a
        // banned member leaves the list, and a timeout ends at its time.
        let governance = self.governance_state_impl(community).unwrap_or_default();
        let banned: std::collections::HashSet<String> =
            governance.bans.iter().map(|p| hex::encode(p.0)).collect();
        let timeout_until = |pseudonym: &str| {
            governance
                .timeouts
                .iter()
                .find(|(p, _)| hex::encode(p.0) == pseudonym)
                .map(|(_, t)| t.started_at.saturating_add(t.duration_seconds))
                .filter(|until| *until > now)
        };
        Ok(rows
            .into_iter()
            .filter(|row| !banned.contains(&row.pseudonym_key))
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
                CommunityMemberView {
                    display_role: display_role_name(&row.role_ids, &overlay.roles),
                    display_name: row.display_name.unwrap_or_default(),
                    role_ids: row.role_ids,
                    status,
                    timeout_until: timeout_until(&row.pseudonym_key),
                    location,
                    last_active,
                    pseudonym_key: row.pseudonym_key,
                    ..profile
                }
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
