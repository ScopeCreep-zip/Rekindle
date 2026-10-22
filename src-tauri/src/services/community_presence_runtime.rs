//! Phase 23.C — presence-handler Tauri-runtime orchestration lifted
//! from `commands/community/presence.rs`. Hosts the four orchestrators
//! (`send_channel_typing_inner`, `update_community_presence_inner`,
//! `get_community_members_inner`, `update_community_profile_inner`).

use rekindle_codec::community::envelope::CommunityEnvelope;

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_db::Db;
use rekindle_types::presence::limits::validate_profile;

pub fn send_channel_typing_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: String,
) -> Result<(), String> {
    let pseudonym_key =
        state_helpers::my_pseudonym_key(state, community_id).ok_or("no pseudonym key")?;

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id,
        pseudonym_key,
    };
    crate::services::community::send_to_mesh(state, community_id, &envelope)
}

pub async fn update_community_presence_inner(
    state: &SharedState,
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
) -> Result<(), String> {
    let pseudonym_key =
        state_helpers::my_pseudonym_key(state, &community_id).ok_or("no pseudonym key")?;

    let game_info = game_name.map(
        |name| rekindle_codec::community::envelope::PresenceGameInfo {
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
        route_blob: state_helpers::our_route_blob(state),
    };
    let gossip_result = crate::services::community::send_to_mesh(state, &community_id, &envelope);
    crate::services::community::write_our_presence(state, &community_id).await;
    gossip_result
}

/// Set (or clear) the local user's focused channel for `community_id`
/// and republish presence so peers see the move within one write.
///
/// `channel_id == None` clears the location (left all channels).
/// `kind` is "voice" for voice channels, anything else → text. The
/// new location rides in `MemberSession.location` on the next write
/// (plaintext in Layer 1; MEK-encrypted extras in Layer 2).
pub async fn set_active_channel_inner(
    state: &SharedState,
    community_id: String,
    channel_id: Option<String>,
    kind: String,
) -> Result<(), String> {
    let location = channel_id.map(|cid| {
        if kind == "voice" {
            rekindle_types::presence::SessionLocation::Voice { channel_id: cid }
        } else {
            rekindle_types::presence::SessionLocation::Text { channel_id: cid }
        }
    });
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(&community_id)
            .ok_or_else(|| "unknown community".to_string())?;
        community.my_session_location = location;
    }
    // Republish immediately so the roster reflects the move now rather
    // than on the next heartbeat tick.
    crate::services::community::write_our_presence(state, &community_id).await;
    Ok(())
}

/// Update the local presence sharing consent (default-deny), persist it
/// to SQLite, and republish immediately so the new redaction takes effect
/// now. The policy is local-only — never written to the registry; it only
/// gates what `write_our_presence` publishes and what the roster reads.
pub async fn set_presence_policy_inner(
    state: &SharedState,
    pool: &Db,
    community_id: String,
    policy: rekindle_types::presence::PresenceSharingPolicy,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(&community_id)
            .ok_or_else(|| "unknown community".to_string())?;
        community.presence_policy = policy.clone();
    }

    let policy_json = serde_json::to_string(&policy).map_err(|e| e.to_string())?;
    let cid_for_db = community_id.clone();
    db_call(pool, move |conn| {
        rekindle_db::repo::communities::set(
            conn,
            &owner_key,
            &cid_for_db,
            rekindle_db::repo::communities::Column::PresencePolicy,
            policy_json,
        )
    })
    .await?;

    // Republish now so opting out of a signal redacts it immediately
    // rather than on the next heartbeat tick.
    crate::services::community::write_our_presence(state, &community_id).await;
    Ok(())
}

/// Read the local presence sharing consent so the settings panel can bind
/// its toggles to the current choice. Defaults (default-deny) when the
/// community isn't loaded.
pub fn get_presence_policy_inner(
    state: &SharedState,
    community_id: &str,
) -> rekindle_types::presence::PresenceSharingPolicy {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|c| c.presence_policy.clone())
        .unwrap_or_default()
}

pub async fn update_community_profile_inner(
    state: &SharedState,
    community_id: String,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
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

    crate::services::community::presence::nudge_presence_poll(state, &community_id);
    Ok(())
}

/// Our own presence changed (the Host's `SelfChanged`): broadcast a
/// `PresenceUpdate` to every joined community over its gossip mesh, so
/// members see our status and game at once (→ E3.4, which carries it on
/// the member's beat). Ephemeral: no durable relay. Invisible appears
/// offline, game included: nothing about it is sent.
pub fn fan_out_own_presence(
    state: &SharedState,
    snapshot: &rekindle_types::subscription_events::PresenceSnapshot,
) {
    use rekindle_types::presence::SessionStatus;
    use rekindle_types::subscription_events::GameActivity;

    let status = snapshot
        .status
        .as_deref()
        .and_then(|s| serde_json::from_value::<SessionStatus>(s.into()).ok())
        .unwrap_or_default();
    if status == SessionStatus::Invisible {
        return;
    }
    let game_info = match &snapshot.game {
        Some(GameActivity::Playing {
            game_name,
            game_id,
            elapsed_seconds,
            server_address,
        }) => Some(rekindle_codec::community::envelope::PresenceGameInfo {
            game_name: game_name.clone(),
            game_id: *game_id,
            elapsed_seconds: *elapsed_seconds,
            server_address: server_address.clone(),
        }),
        Some(GameActivity::Idle) | None => None,
    };
    let community_ids: Vec<String> = state.communities.read().keys().cloned().collect();
    for community_id in community_ids {
        let pseudonym_key =
            state_helpers::my_pseudonym_key(state, &community_id).unwrap_or_default();
        if let Err(e) = crate::services::community::send_to_mesh(
            state,
            &community_id,
            &CommunityEnvelope::PresenceUpdate {
                pseudonym_key,
                status: status.as_wire_str().to_string(),
                game_info: game_info.clone(),
                route_blob: state_helpers::our_route_blob(state),
            },
        ) {
            tracing::debug!(community = %community_id, error = %e, "own presence not fanned out");
        }
    }
}
