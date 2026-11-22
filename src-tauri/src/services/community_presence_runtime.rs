//! Phase 23.C — presence-handler Tauri-runtime orchestration lifted
//! from `commands/community/presence.rs`. Hosts the orchestrators
//! (`update_community_presence_inner`,
//! `get_community_members_inner`, `update_community_profile_inner`).

use rekindle_codec::community::envelope::CommunityEnvelope;

use crate::state::SharedState;
use crate::state_helpers;

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
    let gossip_result = crate::state_helpers::host_governance(state)
        .map(|host| host.send_to_mesh_impl(&community_id, &envelope))
        .map_err(|e| e.to_string());
    crate::services::community::write_our_presence(state, &community_id).await;
    gossip_result
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

/// Our own presence changed (the Host's `SelfChanged`): broadcast a
/// `PresenceUpdate` to every joined community over its gossip mesh, so
/// members see our status and game at once (→ E3.4, which carries it on
/// the member's beat). Ephemeral: no durable relay. A status that hides
/// presence (Invisible, Offline) sends nothing, as a member that quit
/// sends nothing: the Host's rule, [`rekindle_presence::hides_presence`].
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
    if rekindle_presence::hides_presence(status) {
        return;
    }
    let game_info = match &snapshot.game {
        Some(GameActivity::Playing {
            game_name,
            game_id,
            elapsed_seconds,
            server_address,
            ..
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
        let update = CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status: status.as_wire_str().to_string(),
            game_info: game_info.clone(),
            route_blob: state_helpers::our_route_blob(state),
        };
        match crate::state_helpers::host_governance(state) {
            Ok(host) => host.send_to_mesh_impl(&community_id, &update),
            Err(e) => {
                tracing::debug!(community = %community_id, error = %e, "own presence not fanned out");
            }
        }
    }
}
