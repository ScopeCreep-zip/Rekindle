//! The desktop's half of the four Member* control messages (MemberJoined /
//! MemberRemoved / MemberLeave / MemberTimedOut): its in-memory community
//! state, events, raid alert and departure rotations. The roster rows are
//! the Host's (`host/adapters/governance/membership_roster.rs`, plan E3.1
//! D1b), which handles each message first.

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
            display_name,
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
                    pseudonym: pseudonym_key.clone(),
                    display_name,
                    role_ids,
                },
            );

            // Architecture §20.6 — record the join in the per-community
            // sliding window; emit a raid alert if the rate trips the
            // policy threshold.
            let alert = {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    let policy = cs
                        .governance_state
                        .as_ref()
                        .and_then(|gs| gs.community_policy.as_ref())
                        .cloned();
                    rekindle_governance::raid_detection::observe_join(
                        &mut cs.recent_member_joins,
                        rekindle_utils::timestamp_secs(),
                        &pseudonym_key,
                        policy.as_ref(),
                    )
                } else {
                    None
                }
            };
            if let Some(alert) = alert {
                crate::event_dispatch::emit_subscription(
                    app_handle,
                    &rekindle_types::subscription_events::SubscriptionEvent::System(
                        rekindle_types::subscription_events::SystemEvent::RaidDetected {
                            community: community_id.to_string(),
                            joins_in_window: alert.joins_in_window,
                            max_joins_per_interval: alert.max_joins_per_interval,
                            join_interval_seconds: alert.join_interval_seconds,
                        },
                    ),
                );
                tracing::warn!(
                    community = %community_id,
                    joins = alert.joins_in_window,
                    threshold = alert.max_joins_per_interval,
                    interval_s = alert.join_interval_seconds,
                    "raid threshold exceeded — alerting moderators (architecture §20.6)"
                );
            }
        }
        ControlPayload::MemberRemoved { pseudonym_key }
        | ControlPayload::MemberLeave { pseudonym_key } => {
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
        ControlPayload::MemberTimedOut {
            pseudonym_key,
            timeout_until,
        } => {
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::TimeoutStatusChanged {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key,
                    timeout_until,
                },
            );
        }
        _ => {}
    }
}
