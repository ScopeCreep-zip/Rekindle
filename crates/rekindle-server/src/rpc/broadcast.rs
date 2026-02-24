use std::sync::Arc;

use rekindle_protocol::messaging::envelope::CommunityBroadcast;
use rusqlite::params;

use crate::server_state::ServerState;

/// Route target for a broadcast recipient.
struct BroadcastTarget {
    pseudonym_key: String,
    route_blob: Vec<u8>,
}

pub(super) fn broadcast_to_members(
    state: &Arc<ServerState>,
    community_id: &str,
    exclude_pseudonym: &str,
    broadcast: &CommunityBroadcast,
) {
    let broadcast_bytes = serde_json::to_vec(broadcast).unwrap_or_default();

    let targets: Vec<BroadcastTarget> = {
        let hosted = state.hosted.read();
        hosted
            .get(community_id)
            .map(|c| {
                c.members
                    .iter()
                    .filter(|m| m.pseudonym_key_hex != exclude_pseudonym)
                    .filter_map(|m| {
                        m.route_blob.clone().map(|blob| BroadcastTarget {
                            pseudonym_key: m.pseudonym_key_hex.clone(),
                            route_blob: blob,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    for target in targets {
        // Check for IPC broadcast listener first (hosted community owner on same machine)
        let sent_via_ipc = {
            let listeners = state.broadcast_listeners.read();
            if let Some(tx) = listeners.get(&target.pseudonym_key) {
                // Send a newline-delimited JSON line for the broadcast
                let mut ipc_data = broadcast_bytes.clone();
                ipc_data.push(b'\n');
                tx.send(ipc_data).is_ok()
            } else {
                false
            }
        };
        if sent_via_ipc {
            continue; // Skip Veilid path for this target
        }

        let api = state.api.clone();
        let rc = state.routing_context.clone();
        let data = broadcast_bytes.clone();
        let state_clone = state.clone();
        let cid = community_id.to_string();
        tokio::spawn(async move {
            let failed = match api.import_remote_private_route(target.route_blob) {
                Ok(route_id) => {
                    if let Err(e) = rc
                        .app_message(veilid_core::Target::RouteId(route_id), data)
                        .await
                    {
                        let msg = e.to_string();
                        // Only mark offline for definitive route failures, not transient timeouts
                        let is_permanent =
                            msg.contains("InvalidTarget") || msg.contains("NoConnection");
                        if is_permanent {
                            tracing::info!(
                                member = %target.pseudonym_key,
                                error = %e,
                                "broadcast failed permanently, marking member offline"
                            );
                            true
                        } else {
                            tracing::debug!(error = %e, "transient broadcast failure to member");
                            false
                        }
                    } else {
                        false
                    }
                }
                Err(e) => {
                    tracing::info!(
                        member = %target.pseudonym_key,
                        error = %e,
                        "failed to import member route, marking offline"
                    );
                    true
                }
            };

            if failed {
                mark_member_offline(&state_clone, &cid, &target.pseudonym_key);
            }
        });
    }
}

/// Update a member's status to "offline" and broadcast the change to remaining members.
fn mark_member_offline(state: &Arc<ServerState>, community_id: &str, pseudonym_key: &str) {
    {
        let mut hosted = state.hosted.write();
        let Some(community) = hosted.get_mut(community_id) else {
            return;
        };
        let Some(member) = community.find_member_mut(pseudonym_key) else {
            return;
        };
        if member.online_status == "offline" {
            return;
        }
        member.online_status = "offline".to_string();
        // Clear stale route_blob so we stop wasting broadcast attempts on a dead route.
        // The member will re-announce their route via a rejoin RPC when they reconnect.
        member.route_blob = None;
    }

    // Persist the cleared route to DB
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    crate::db_helpers::db_fire(&state.db, "clear dead member route", |db| {
        db.execute(
            "UPDATE server_members SET route_blob = NULL, online_status = 'offline' \
             WHERE community_id = ? AND pseudonym_key_hex = ?",
            params![cid, pk],
        )?;
        Ok(())
    });

    // Broadcast the offline status to remaining members
    broadcast_to_members(
        state,
        community_id,
        pseudonym_key,
        &CommunityBroadcast::MemberPresenceChanged {
            community_id: community_id.to_string(),
            pseudonym_key: pseudonym_key.to_string(),
            status: "offline".to_string(),
            game_name: None,
            game_id: None,
            elapsed_seconds: None,
            server_address: None,
        },
    );
}

pub(super) fn broadcast_roles_changed(state: &Arc<ServerState>, community_id: &str) {
    use super::permissions::roles_to_dto;

    let roles = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        roles_to_dto(community)
    };

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::RolesChanged {
            community_id: community_id.to_string(),
            roles,
        },
    );
}

pub(super) fn broadcast_member_roles_changed(
    state: &Arc<ServerState>,
    community_id: &str,
    target_pseudonym: &str,
) {
    let role_ids = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        community
            .find_member(target_pseudonym)
            .map_or_else(Vec::new, |m| m.role_ids.clone())
    };

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::MemberRolesChanged {
            community_id: community_id.to_string(),
            pseudonym_key: target_pseudonym.to_string(),
            role_ids,
        },
    );
}

/// Broadcast an event reminder to all members of a community.
pub fn broadcast_event_reminder(
    state: &Arc<ServerState>,
    community_id: &str,
    event_id: &str,
    title: &str,
    minutes_until_start: u32,
) {
    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::EventReminder {
            community_id: community_id.to_string(),
            event_id: event_id.to_string(),
            title: title.to_string(),
            minutes_until_start,
        },
    );
}
