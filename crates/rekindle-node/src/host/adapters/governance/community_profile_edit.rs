//! Applying and storing edits to our per-community profile.

use rekindle_db::repo::members::OwnProfile;
use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};

use super::HostGovernance;
use crate::host::community::state::CommunityState;

fn own_profile(state: &CommunityState) -> OwnProfile {
    OwnProfile {
        bio: state.my_bio.clone(),
        pronouns: state.my_pronouns.clone(),
        theme_color: state.my_theme_color,
        badges: state.my_badges.clone(),
        avatar_ref: state.my_avatar_ref.clone(),
        banner_ref: state.my_banner_ref.clone(),
    }
}

fn edit_profile(edit: CommunityProfilePatch, mut profile: OwnProfile) -> OwnProfile {
    edit.bio.apply_to(&mut profile.bio);
    edit.pronouns.apply_to(&mut profile.pronouns);
    edit.theme_color.apply_to(&mut profile.theme_color);
    edit.avatar_ref.apply_to(&mut profile.avatar_ref);
    edit.banner_ref.apply_to(&mut profile.banner_ref);
    match edit.badges {
        ProfileField::Unchanged => {}
        ProfileField::Clear => profile.badges.clear(),
        ProfileField::Set(badges) => profile.badges = badges,
    }
    profile
}

impl HostGovernance {
    /// Apply our explicit local profile and published card edits.
    ///
    /// # Errors
    /// The community is not joined, or a profile or card write failed.
    pub(crate) async fn update_profile(
        &self,
        community: &str,
        mut patch: CommunityProfilePatch,
    ) -> Result<(), String> {
        let guard = self.ctx.communities.profile_write_lock(community);
        let _held = guard.lock().await;
        let display_name = std::mem::take(&mut patch.display_name);
        let custom_status = std::mem::take(&mut patch.custom_status);
        self.store_profile_edit(community, patch).await?;
        if display_name.is_unchanged() && custom_status.is_unchanged() {
            return Ok(());
        }
        let mut text = self.own_card_text(community);
        match display_name {
            ProfileField::Unchanged => {}
            ProfileField::Clear => text.display_name = None,
            ProfileField::Set(name) => text.display_name = Some(name.trim().to_owned()),
        }
        custom_status.apply_to(&mut text.custom_status);
        self.publish_own_card(community, &text).await
    }

    /// Apply the Overview's edit through the same card path as the profile form.
    pub(crate) async fn set_display_name(
        &self,
        community: &str,
        display_name: ProfileField<String>,
    ) -> Result<(), String> {
        self.update_profile(
            community,
            CommunityProfilePatch {
                display_name,
                ..CommunityProfilePatch::default()
            },
        )
        .await
    }

    async fn store_profile_edit(
        &self,
        community: &str,
        patch: CommunityProfilePatch,
    ) -> Result<(), String> {
        let (old, pseudonym) = {
            let map = self.ctx.communities.map().read();
            let state = map.get(community).ok_or("unknown community")?;
            let pseudonym = state.my_pseudonym_key.clone().ok_or("no pseudonym")?;
            (own_profile(state), pseudonym)
        };
        let profile = edit_profile(patch, old.clone());
        if profile == old {
            return Ok(());
        }

        let (owner, db) = self.owner_and_db().ok_or("no identity is unlocked")?;
        let (key, me, row) = (community.to_owned(), pseudonym.clone(), profile.clone());
        db.call(move |conn| {
            rekindle_db::repo::members::set_own_profile(conn, &owner, &key, &me, &row)
        })
        .await
        .map_err(|e| format!("profile not stored: {e}"))?;

        let mut map = self.ctx.communities.map().write();
        let state = map.get_mut(community).ok_or("unknown community")?;
        if state.my_pseudonym_key.as_deref() != Some(pseudonym.as_str()) {
            return Err("community profile owner changed".to_owned());
        }
        state.my_bio = profile.bio;
        state.my_pronouns = profile.pronouns;
        state.my_theme_color = profile.theme_color;
        state.my_badges = profile.badges;
        state.my_avatar_ref = profile.avatar_ref;
        state.my_banner_ref = profile.banner_ref;
        Ok(())
    }
}
