use std::sync::Arc;

use crate::state::AppState;
use rekindle_db::Db;

use super::control_events::handle_control_events_and_threads;
use crate::services::governance_adapter;

pub(crate) async fn handle_relayed_control(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        payload @ (ControlPayload::MemberJoinRequest { .. }
        | ControlPayload::MemberJoined { .. }
        | ControlPayload::MemberRemoved { .. }
        | ControlPayload::MemberLeave { .. }
        | ControlPayload::MemberTimedOut { .. }) => {
            super::control_membership::handle_membership_payload(
                app_handle,
                state,
                pool,
                community_id,
                payload,
            );
        }
        payload @ (ControlPayload::MessageEdited { .. }
        | ControlPayload::MessageDeleted { .. }
        | ControlPayload::ReactionAdded { .. }
        | ControlPayload::ReactionRemoved { .. }) => {
            handle_channel_event_payload(app_handle, state, community_id, payload);
        }
        payload @ (ControlPayload::MemberRolesChanged { .. }
        | ControlPayload::JoinAccepted { .. }
        | ControlPayload::JoinRejected { .. }
        | ControlPayload::RequestMEK { .. }
        | ControlPayload::RequestSegmentExpansion { .. }) => {
            handle_join_and_roles_payload(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                payload,
            )
            .await;
        }
        other => {
            handle_control_events_and_threads(
                app_handle,
                state,
                pool,
                community_id,
                sender_pseudonym,
                other,
            )
            .await;
        }
    }
}

/// An edit's new body, opened under exactly the generation of the
/// channel's text key the edit names.
fn decrypt_edited_message_body(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    mek_generation: u64,
    new_ciphertext: &[u8],
) -> String {
    let key = crate::state_helpers::text_scope(state, community_id, channel_id).and_then(|scope| {
        crate::state_helpers::key_provider(state).key(
            community_id,
            scope,
            rekindle_types::channel_keys::KeyEpoch(mek_generation),
        )
    });
    let Some(key) = key else {
        return "(no MEK available)".to_string();
    };
    match rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*key, mek_generation)
        .decrypt(new_ciphertext)
    {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => "(decryption failed)".to_string(),
    }
}

fn handle_channel_event_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MessageEdited {
            channel_id,
            message_id,
            new_ciphertext,
            mek_generation,
            edited_at,
        } => {
            let new_body = decrypt_edited_message_body(
                state,
                community_id,
                &channel_id,
                mek_generation,
                &new_ciphertext,
            );
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                    rekindle_types::subscription_events::ChannelMessageEvent::Edited {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        body: Some(new_body),
                        edited_at,
                    },
                ),
            );
        }
        ControlPayload::MessageDeleted {
            channel_id,
            message_id,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                    rekindle_types::subscription_events::ChannelMessageEvent::Deleted {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                    },
                ),
            );
        }
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ReactionAdded {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        emoji,
                        reactor_pseudonym,
                    },
                ),
            );
        }
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ReactionRemoved {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        emoji,
                        reactor_pseudonym,
                    },
                ),
            );
        }
        _ => {}
    }
}

async fn handle_join_and_roles_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MemberRolesChanged {
            pseudonym_key,
            role_ids,
        } => {
            governance_adapter::process_member_roles_changed(
                state,
                app_handle,
                community_id,
                &pseudonym_key,
                &role_ids,
            );
        }
        ControlPayload::JoinAccepted {
            mek_encrypted,
            mek_generation,
            members,
            member_registry_key,
            slot_index,
            wrapped_slot_seed,
        } => {
            let input = rekindle_governance_runtime::membership_events::JoinAcceptedInput {
                mek_wire_bytes: &mek_encrypted,
                mek_generation,
                members: &members,
                member_registry_key: member_registry_key.as_deref(),
                slot_index,
                wrapped_slot_seed: wrapped_slot_seed.as_deref(),
            };
            governance_adapter::process_join_accepted(
                state,
                app_handle,
                community_id,
                sender_pseudonym,
                input,
            )
            .await;
        }
        ControlPayload::JoinRejected { reason } => {
            tracing::warn!(community = %community_id, reason = %reason, "join request rejected by peer");
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::JoinRejected {
                    community: community_id.to_string(),
                    reason,
                },
            );
        }
        ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            cascade_index,
        } => {
            if let Err(error) = crate::services::community::handle_request_mek(
                app_handle,
                state,
                community_id,
                &channel_id,
                needed_generation,
                &requester_pseudonym,
                cascade_index,
            )
            .await
            {
                tracing::debug!(community = %community_id, error = %error, "RequestMEK ignored");
            }
        }
        ControlPayload::RequestSegmentExpansion {
            community_id: req_community_id,
            requester_pseudonym,
            full_segment_index,
        } => {
            // P4.3 — admin handler. Any peer with MANAGE_COMMUNITY who
            // sees this gossip request expands the community by one
            // segment. The first admin's `expand_community_segment`
            // call wins via CRDT — later admins see `next_segment_index
            // > full_segment_index + 1` in their merged state and the
            // pre-flight check inside `expand_community_segment`
            // (`highest_segment_full`) returns false, no-op'ing the
            // expansion.
            //
            // Defense-in-depth: ignore the request if the community_id
            // doesn't match the gossip envelope's community (a forged
            // envelope can't redirect us).
            if req_community_id != community_id {
                tracing::debug!(
                    envelope_cid = %community_id,
                    req_cid = %req_community_id,
                    "RequestSegmentExpansion ignored: community_id mismatch"
                );
                return;
            }
            // Permission check: only act if we have MANAGE_COMMUNITY.
            // Other peers without the bit ignore — that's the spec's
            // "any admin reacts" model.
            let have_manage_community = rekindle_governance::permissions::has_capability(
                crate::state_helpers::my_permissions(state, community_id, None),
                rekindle_types::permissions::MANAGE_COMMUNITY,
            );
            if !have_manage_community {
                tracing::debug!(
                    community = %community_id,
                    requester = %requester_pseudonym,
                    full_segment_index,
                    "RequestSegmentExpansion ignored: we lack MANAGE_COMMUNITY"
                );
                return;
            }
            tracing::info!(
                community = %community_id,
                requester = %requester_pseudonym,
                full_segment_index,
                "RequestSegmentExpansion received — calling expand_community_segment"
            );
            if let Err(error) =
                crate::services::community::segments::expand_community_segment(state, community_id)
                    .await
            {
                // The "current segment still has open slots" + "segment
                // cap reached" errors are expected race-conditions when
                // multiple admins receive the same request. Log at
                // debug level to avoid alarm-fatigue.
                tracing::debug!(
                    community = %community_id,
                    error = %error,
                    "expand_community_segment did not run (expected on concurrent admin races)"
                );
            }
        }
        _ => {}
    }
}
