use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
// Voice signaling dispatch now goes through
// `crate::services::voice_signaling_adapter::handle_voice_signaling`.
use crate::state::AppState;
use tauri::Manager;

use super::control_sync::{
    check_gossip_moderation_permission, handle_sync_request, handle_sync_response,
};
use crate::services::veilid::legacy::membership::{
    handle_admin_keypair_grant, handle_slot_keypair_grant,
};

pub(crate) fn handle_gossip_control_payloads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::AdminKeypairGrant {
            wrapped_owner_keypair,
            wrapped_slot_seed,
        } => {
            handle_admin_keypair_grant(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                &wrapped_owner_keypair,
                &wrapped_slot_seed,
            );
        }
        ControlPayload::SlotKeypairGrant {
            slot_index,
            segment_index,
            wrapped_slot_keypair,
        } => {
            handle_slot_keypair_grant(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                slot_index,
                segment_index,
                &wrapped_slot_keypair,
            );
        }
        ControlPayload::SyncRequest {
            channel_id,
            since_timestamp,
        } => {
            handle_sync_request(
                app_handle,
                state,
                community_id,
                &channel_id,
                since_timestamp,
            );
        }
        ControlPayload::SyncResponse {
            channel_id,
            messages,
        } => {
            {
                let mut communities = state.communities.write();
                if let Some(cs) = communities.get_mut(community_id) {
                    cs.pending_syncs.remove(&channel_id);
                }
            }
            handle_sync_response(app_handle, state, community_id, &channel_id, &messages);
        }
        ControlPayload::GovernanceUpdated {
            governance_key,
            subkey_index: _,
            lamport_ts: _,
        } => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let db_pool = pool.inner().clone();
            let state = Arc::clone(state);
            tokio::spawn(async move {
                let _ = crate::services::sync_communities::handle_community_record_change(
                    &state,
                    &db_pool,
                    &governance_key,
                )
                .await;
            });
        }
        ControlPayload::VoiceJoin { .. }
        | ControlPayload::VoiceLeave { .. }
        | ControlPayload::VoiceModeSwitch { .. }
        | ControlPayload::StageUpdate { .. }
        | ControlPayload::SpeakRequest { .. }
        | ControlPayload::SpeakResponse { .. }
        | ControlPayload::VoiceMute { .. }
        | ControlPayload::VoiceDeafen { .. }
        | ControlPayload::VoiceRoster { .. }
        | ControlPayload::SoundboardPlay { .. } => {
            // Voice signaling adapter handles the spawn-and-forget
            // dispatch internally — the gossip dispatcher stays sync.
            crate::services::voice_signaling_adapter::handle_voice_signaling(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            );
        }
        ControlPayload::VideoFragment { .. }
        | ControlPayload::VideoParityFragment { .. }
        | ControlPayload::FrameAck { .. }
        | ControlPayload::KeyframeRequest { .. }
        | ControlPayload::BandwidthEstimate { .. }
        | ControlPayload::TopologyChange { .. }
        | ControlPayload::MediaCapabilities { .. } => {
            crate::services::community::video::handle_video_payload(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            );
        }
        ControlPayload::LinkPreview {
            channel_id,
            message_id,
            url,
            title,
            description,
            image_url,
            site_name,
            fetched_at,
        } => {
            crate::services::community::link_previews::handle_incoming_link_preview(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                channel_id,
                message_id,
                url,
                title,
                description,
                image_url,
                site_name,
                fetched_at,
            );
        }
        other => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            handle_gossip_moderation(
                app_handle,
                state,
                &pool,
                community_id,
                sender_pseudonym,
                other,
            );
        }
    }
}

fn handle_gossip_moderation(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    let Ok(owner_key) = crate::state_helpers::current_owner_key(state) else {
        return;
    };

    if !check_gossip_moderation_permission(state, community_id, sender_pseudonym, &payload) {
        return;
    }

    match payload {
        ControlPayload::Kick { target_pseudonym } => remove_member_from_local_state(
            app_handle,
            state,
            pool,
            community_id,
            owner_key,
            target_pseudonym,
            "kick_member_remove",
        ),
        ControlPayload::Ban {
            target_pseudonym, ..
        } => {
            remove_member_from_local_state(
                app_handle,
                state,
                pool,
                community_id,
                owner_key,
                target_pseudonym.clone(),
                "ban_member_remove",
            );
            let state = state.clone();
            let app_handle = app_handle.clone();
            let community_id = community_id.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::services::community::rotate_text_mek_for_departure(
                    &app_handle,
                    &state,
                    &community_id,
                    &target_pseudonym,
                )
                .await
                {
                    tracing::debug!(community = %community_id, member = %target_pseudonym, error = %error, "text MEK rotation skipped after ban");
                }
            });
        }
        ControlPayload::Unban { .. } => {}
        ControlPayload::TimeoutMember {
            target_pseudonym,
            duration_seconds,
            ..
        } => {
            let timeout_until = rekindle_utils::timestamp_secs() + duration_seconds;
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "timeout_member", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                    rusqlite::params![timeout_until, ok, cid, tp],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    timeout_until: Some(timeout_until),
                },
            );
        }
        ControlPayload::RemoveTimeout { target_pseudonym } => {
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let tp = target_pseudonym.clone();
            db_fire(pool, "remove_timeout", move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = NULL \
                     WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![ok, cid, tp],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::MemberTimedOut {
                    community_id: community_id.to_string(),
                    pseudonym_key: target_pseudonym,
                    timeout_until: None,
                },
            );
        }
        other => {
            tracing::trace!(
                community = %community_id,
                payload = ?other,
                "received unhandled moderation/structural payload"
            );
        }
    }
}

fn remove_member_from_local_state(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    owner_key: String,
    target_pseudonym: String,
    label: &'static str,
) {
    use crate::channels::CommunityEvent;

    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };

    if my_pseudonym.as_deref() == Some(&target_pseudonym) {
        crate::event_dispatch::emit_live(
            app_handle,
            "community-event",
            &CommunityEvent::Kicked {
                community_id: community_id.to_string(),
            },
        );
        return;
    }

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.known_members.remove(&target_pseudonym);
            if let Some(ref mut gossip) = cs.gossip {
                gossip.online_members.remove(&target_pseudonym);
                gossip.peers.remove(&target_pseudonym);
            }
        }
    }
    crate::services::community::analytics::log_member_leave(
        pool,
        &owner_key,
        community_id,
        &target_pseudonym,
    );
    let cid = community_id.to_string();
    let tp = target_pseudonym.clone();
    db_fire(pool, label, move |conn| {
        conn.execute(
            "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
            rusqlite::params![owner_key, cid, tp],
        )?;
        Ok(())
    });
    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &CommunityEvent::MemberRemoved {
            community_id: community_id.to_string(),
            pseudonym_key: target_pseudonym,
        },
    );
}
