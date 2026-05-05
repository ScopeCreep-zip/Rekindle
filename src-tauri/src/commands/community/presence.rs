use tauri::State;

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use super::types::MemberDto;

const MAX_BIO_LEN: usize = 190;
/// Architecture §24.2 specifies pronouns ≤40 chars.
const MAX_PRONOUNS_LEN: usize = 40;
const MAX_BADGES: usize = 8;
const MAX_BADGE_LEN: usize = 32;
/// blake3 content-hash hex (64 chars) is the canonical avatar/banner
/// reference per architecture §24.2; raw bytes never appear in the
/// MemberPresence record. Anything beyond hex+small-prefix scheme
/// pollutes the DHT subkey and bloats presence updates.
const MAX_CONTENT_REF_LEN: usize = 96;

fn validate_profile(
    bio: Option<&str>,
    pronouns: Option<&str>,
    badges: &[String],
    avatar_ref: Option<&str>,
    banner_ref: Option<&str>,
) -> Result<(), String> {
    if let Some(b) = bio {
        if b.chars().count() > MAX_BIO_LEN {
            return Err(format!("bio exceeds {MAX_BIO_LEN} characters"));
        }
    }
    if let Some(p) = pronouns {
        if p.chars().count() > MAX_PRONOUNS_LEN {
            return Err(format!("pronouns exceeds {MAX_PRONOUNS_LEN} characters"));
        }
    }
    if badges.len() > MAX_BADGES {
        return Err(format!("badges count exceeds {MAX_BADGES}"));
    }
    if badges.iter().any(|b| b.chars().count() > MAX_BADGE_LEN) {
        return Err(format!("badge exceeds {MAX_BADGE_LEN} characters"));
    }
    if let Some(a) = avatar_ref {
        if a.chars().count() > MAX_CONTENT_REF_LEN {
            return Err(format!("avatar_ref exceeds {MAX_CONTENT_REF_LEN} characters"));
        }
    }
    if let Some(b) = banner_ref {
        if b.chars().count() > MAX_CONTENT_REF_LEN {
            return Err(format!("banner_ref exceeds {MAX_CONTENT_REF_LEN} characters"));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id,
        pseudonym_key,
    };
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

#[tauri::command]
pub async fn update_community_presence(
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let game_info =
        game_name.map(
            |name| rekindle_protocol::dht::community::envelope::PresenceGameInfo {
                game_name: name,
                game_id,
                elapsed_seconds,
                server_address: server_address.clone(),
            },
        );

    let envelope = CommunityEnvelope::PresenceUpdate {
        pseudonym_key,
        status,
        game_info,
        route_blob: crate::state_helpers::our_route_blob(state.inner()),
    };
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<MemberDto>, String> {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };
    let my_status =
        state_helpers::identity_status(state.inner()).unwrap_or(crate::state::UserStatus::Online);

    let (role_defs, online_statuses, member_profiles, my_profile) = {
        let communities = state.communities.read();
        communities.get(&community_id).map_or_else(
            || {
                (
                    Vec::new(),
                    std::collections::HashMap::new(),
                    std::collections::HashMap::new(),
                    None,
                )
            },
            |c| {
                let online = c
                    .gossip
                    .as_ref()
                    .map(|g| {
                        g.online_members
                            .iter()
                            .map(|(pk, member)| (pk.clone(), member.status.clone()))
                            .collect::<std::collections::HashMap<_, _>>()
                    })
                    .unwrap_or_default();
                let mine = crate::state::MemberProfileSnapshot {
                    display_name: state
                        .identity
                        .read()
                        .as_ref()
                        .map(|id| id.display_name.clone()),
                    bio: c.my_bio.clone(),
                    pronouns: c.my_pronouns.clone(),
                    theme_color: c.my_theme_color,
                    badges: c.my_badges.clone(),
                    avatar_ref: c.my_avatar_ref.clone(),
                    banner_ref: c.my_banner_ref.clone(),
                };
                (c.roles.clone(), online, c.member_profiles.clone(), Some(mine))
            },
        )
    };

    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_clone = community_id.clone();
    let members = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name, role_ids, timeout_until FROM community_members \
                 WHERE owner_key = ? AND community_id = ? ORDER BY display_name",
        )?;

        let rows = stmt.query_map(rusqlite::params![owner_key, community_id_clone], |row| {
            let pseudonym_key = db::get_str(row, "pseudonym_key");
            let status_str = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                match my_status {
                    crate::state::UserStatus::Online => "online",
                    crate::state::UserStatus::Away => "away",
                    crate::state::UserStatus::Busy => "busy",
                    crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => {
                        "offline"
                    }
                }
            } else {
                online_statuses
                    .get(&pseudonym_key)
                    .map_or("offline", String::as_str)
            };

            let role_ids_json = db::get_str(row, "role_ids");
            let role_ids: Vec<u32> =
                serde_json::from_str(&role_ids_json).unwrap_or_else(|_| vec![0, 1]);
            let display_role = crate::state::display_role_name(&role_ids, &role_defs);
            let timeout_until: Option<u64> = row
                .get::<_, Option<i64>>("timeout_until")
                .ok()
                .flatten()
                .map(i64::cast_unsigned);

            let profile = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                my_profile.clone()
            } else {
                member_profiles.get(&pseudonym_key).cloned()
            };
            let snap = profile.unwrap_or_default();

            Ok(MemberDto {
                pseudonym_key,
                display_name: db::get_str(row, "display_name"),
                role_ids,
                display_role,
                status: status_str.to_string(),
                timeout_until,
                bio: snap.bio,
                pronouns: snap.pronouns,
                theme_color: snap.theme_color,
                badges: snap.badges,
                avatar_ref: snap.avatar_ref,
                banner_ref: snap.banner_ref,
            })
        })?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row?);
        }
        Ok(members)
    })
    .await?;

    Ok(members)
}

/// Update our per-community profile (bio, pronouns, theme color, badges).
///
/// The four fields are stored on local `CommunityState` and propagated to
/// peers via the next presence write (which the periodic poll triggers within
/// at most ~60s; the rapid-tick window after join makes the first publish
/// faster). The same identity (master secret) can present a different
/// persona per community without identity-linkability across communities,
/// because each community uses a distinct pseudonym derivation.
#[tauri::command]
pub async fn update_community_profile(
    community_id: String,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    validate_profile(
        bio.as_deref(),
        pronouns.as_deref(),
        &badges,
        avatar_ref.as_deref(),
        banner_ref.as_deref(),
    )?;
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(&community_id)
            .ok_or_else(|| "unknown community".to_string())?;
        community.my_bio = bio;
        community.my_pronouns = pronouns;
        community.my_theme_color = theme_color;
        community.my_badges = badges;
        community.my_avatar_ref = avatar_ref;
        community.my_banner_ref = banner_ref;
    }

    // Plan §Failure 7 — nudge the periodic presence loop so peers see
    // the new profile fields within ~1 s instead of waiting up to
    // 30 s for the next scheduled poll. Best-effort: failures here
    // mean the profile arrives at peers on the next cycle (fine).
    if let Err(e) = crate::services::community::presence::presence_poll_tick_public(
        state.inner(),
        &community_id,
    )
    .await
    {
        tracing::debug!(
            community = %community_id,
            error = %e,
            "profile-update presence nudge skipped",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_profile_accepts_empty_inputs() {
        assert!(validate_profile(None, None, &[], None, None).is_ok());
    }

    #[test]
    fn validate_profile_accepts_boundary_inputs() {
        let bio = "x".repeat(MAX_BIO_LEN);
        let pronouns = "y".repeat(MAX_PRONOUNS_LEN);
        let badges: Vec<String> = (0..MAX_BADGES)
            .map(|_| "z".repeat(MAX_BADGE_LEN))
            .collect();
        let avatar = "a".repeat(MAX_CONTENT_REF_LEN);
        let banner = "b".repeat(MAX_CONTENT_REF_LEN);
        assert!(validate_profile(
            Some(&bio),
            Some(&pronouns),
            &badges,
            Some(&avatar),
            Some(&banner)
        )
        .is_ok());
    }

    #[test]
    fn validate_profile_rejects_oversized_bio() {
        let bio = "x".repeat(MAX_BIO_LEN + 1);
        assert!(validate_profile(Some(&bio), None, &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_rejects_oversized_pronouns() {
        let pronouns = "y".repeat(MAX_PRONOUNS_LEN + 1);
        assert!(validate_profile(None, Some(&pronouns), &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_rejects_too_many_badges() {
        let badges: Vec<String> = (0..=MAX_BADGES).map(|_| "a".to_string()).collect();
        assert!(validate_profile(None, None, &badges, None, None).is_err());
    }

    #[test]
    fn validate_profile_rejects_oversized_badge() {
        let badges = vec!["z".repeat(MAX_BADGE_LEN + 1)];
        assert!(validate_profile(None, None, &badges, None, None).is_err());
    }

    #[test]
    fn validate_profile_counts_unicode_chars_not_bytes() {
        // Each emoji is 4 bytes but 1 char. MAX_BIO_LEN emoji should pass.
        let bio: String = "🔥".repeat(MAX_BIO_LEN);
        assert!(validate_profile(Some(&bio), None, &[], None, None).is_ok());
        // One extra emoji should fail.
        let bio_over: String = "🔥".repeat(MAX_BIO_LEN + 1);
        assert!(validate_profile(Some(&bio_over), None, &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_pronouns_now_allow_40_chars() {
        // Architecture §24.2 raised the cap from 32 to 40.
        let pronouns = "y".repeat(40);
        assert!(validate_profile(None, Some(&pronouns), &[], None, None).is_ok());
        let too_long = "y".repeat(41);
        assert!(validate_profile(None, Some(&too_long), &[], None, None).is_err());
    }

    #[test]
    fn validate_profile_rejects_oversized_avatar_ref() {
        let oversized = "a".repeat(MAX_CONTENT_REF_LEN + 1);
        assert!(validate_profile(None, None, &[], Some(&oversized), None).is_err());
    }
}
