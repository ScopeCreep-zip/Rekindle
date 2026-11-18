//! The desktop's half of a member's own `MemberJoined` and `MemberLeave`
//! notices: the gossip peer hint a join carries (until E3.4's beat) and
//! the departure rotations a leave triggers (until E3.5 moves them onto the
//! merged leave). The roster rows are the Host's
//! (`host/adapters/governance/membership_roster.rs`, plan E3.1 D1b), which
//! handles each message first; the raid alert counts merged joins
//! (`host/adapters/governance/member_events.rs`).

use std::sync::Arc;

use crate::state::AppState;

pub(super) fn handle_membership_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MemberJoined {
            pseudonym_key,
            // A gossip notice's name is a hint, not the member's verified
            // card (plan E3.2 F9-2), so the Joined below carries none.
            display_name: _,
            role_ids,
            status,
            route_blob,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.insert(pseudonym_key.clone());
                }
            }

            if status != "offline" {
                if let Some(ref blob) = route_blob {
                    if !blob.is_empty() {
                        let mut communities = state.communities.write();
                        if let Some(cs) = communities.get_mut(community_id) {
                            if cs.gossip.is_none() {
                                cs.gossip = Some(crate::state::GossipOverlay::default());
                            }
                            if let Some(ref mut gossip) = cs.gossip {
                                let member = crate::state::OnlineMember {
                                    route_blob: blob.clone(),
                                    status: status.clone(),
                                    last_seen: rekindle_utils::timestamp_secs(),
                                    ..Default::default()
                                };
                                gossip
                                    .online_members
                                    .insert(pseudonym_key.clone(), member.clone());
                                gossip.peers.insert(pseudonym_key.clone(), member);
                            }
                        }
                    }
                }
            }
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::Joined {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key,
                    display_name: None,
                    role_ids: role_ids.iter().map(|id| id.to_hex()).collect(),
                },
            );
        }
        ControlPayload::MemberLeave { pseudonym_key } => {
            let departed_pseudonym = pseudonym_key.clone();
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.known_members.remove(&pseudonym_key);
                    if let Some(ref mut gossip) = cs.gossip {
                        gossip.online_members.remove(&pseudonym_key);
                        gossip.peers.remove(&pseudonym_key);
                    }
                }
            }

            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::Removed {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key,
                },
            );

            crate::services::community::spawn_departure_rotations(
                app_handle,
                state,
                community_id,
                &departed_pseudonym,
            );
        }
        _ => {}
    }
}
