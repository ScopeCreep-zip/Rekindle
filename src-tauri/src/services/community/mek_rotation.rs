use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::group::mek_distribution::unwrap_mek;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::rotator::{cascade_candidates, select_mek_responder};
use rekindle_types::governance::GovernanceEntry;

use super::mek_rotation_support::{
    current_generation, distribute_mek, emit_rotation_event, lookup_mek, max_cascades,
    my_pseudonym, my_pseudonym_hex, online_recipients, persist_mek, pseudonym_from_hex,
    update_generation_state, voice_recipients, wait_for_rotation_slot,
};

fn selected_request_responder(
    state: &Arc<AppState>,
    community_id: &str,
    requester_pseudonym: &str,
    candidates: &[String],
) -> Result<bool, String> {
    let mut candidate_keys = candidates
        .iter()
        .filter_map(|pseudonym| pseudonym_from_hex(pseudonym))
        .collect::<Vec<_>>();
    if let Some(me) = my_pseudonym(state, community_id) {
        candidate_keys.push(me);
    }
    let requester = pseudonym_from_hex(requester_pseudonym).ok_or("invalid requester pseudonym")?;
    let Some(selected) = select_mek_responder(&requester, &candidate_keys) else {
        return Ok(false);
    };
    Ok(my_pseudonym(state, community_id).as_ref() == Some(&selected))
}

fn apply_received_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &MediaEncryptionKey,
) {
    let generation = mek.generation();
    match channel_id {
        Some(channel_id) if !channel_id.is_empty() => {
            state.channel_mek_cache.lock().insert(
                (community_id.to_string(), channel_id.to_string()),
                mek.clone(),
            );
            update_generation_state(state, community_id, Some(channel_id), generation);
        }
        _ => {
            state
                .mek_cache
                .lock()
                .insert(community_id.to_string(), mek.clone());
            update_generation_state(state, community_id, None, generation);
        }
    }
}

fn unwrap_received_mek(
    community_id: &str,
    recipient_secret: &[u8; 32],
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<MediaEncryptionKey, String> {
    let my_signing_key = rekindle_crypto::group::pseudonym::derive_community_pseudonym(
        recipient_secret,
        community_id,
    );
    let sender_bytes =
        hex::decode(sender_pseudonym).map_err(|e| format!("invalid sender pseudonym hex: {e}"))?;
    let sender_pub: [u8; 32] = sender_bytes
        .try_into()
        .map_err(|_| "sender pseudonym must be 32 bytes")?;
    let mek_wire = unwrap_mek(&my_signing_key, &sender_pub, wrapped_mek)
        .map_err(|e| format!("unwrap MEK failed: {e}"))?;
    MediaEncryptionKey::from_wire_bytes(&mek_wire).ok_or("invalid MEK wire bytes".into())
}

pub async fn rotate_text_mek_for_departure(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed_pseudonym: &str,
) -> Result<(), String> {
    let recipients = online_recipients(state, community_id, Some(departed_pseudonym));
    let departed = pseudonym_from_hex(departed_pseudonym).ok_or("invalid departed pseudonym")?;
    let candidates = cascade_candidates(
        &departed,
        &recipients
            .iter()
            .filter_map(|recipient| pseudonym_from_hex(&recipient.pseudonym_hex))
            .collect::<Vec<_>>(),
        max_cascades(),
    );
    let Some(cascade_skipped) =
        wait_for_rotation_slot(state, community_id, None, &candidates).await
    else {
        return Ok(());
    };

    let new_generation = current_generation(state, community_id, None) + 1;
    let mek = MediaEncryptionKey::generate(new_generation);
    distribute_mek(state, community_id, None, &mek, &recipients, false).await?;

    state
        .mek_cache
        .lock()
        .insert(community_id.to_string(), mek.clone());
    update_generation_state(state, community_id, None, new_generation);
    persist_mek(app_handle, community_id, None, &mek);

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        GovernanceEntry::MEKGenerationBump {
            generation: new_generation,
            trigger_departed: departed,
            cascade_skipped,
            lamport,
        },
    )
    .await?;

    emit_rotation_event(app_handle, community_id, None, new_generation);
    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: None,
            new_generation,
            rotator_pseudonym: my_pseudonym_hex(state, community_id),
        }),
    )?;
    Ok(())
}

pub async fn rotate_voice_mek_for_membership(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    trigger_pseudonym: &str,
    include_trigger_in_recipients: bool,
) -> Result<(), String> {
    let recipients = voice_recipients(
        state,
        community_id,
        channel_id,
        trigger_pseudonym,
        include_trigger_in_recipients,
    );
    let trigger = pseudonym_from_hex(trigger_pseudonym).ok_or("invalid trigger pseudonym")?;
    let candidates = cascade_candidates(
        &trigger,
        &recipients
            .iter()
            .filter(|recipient| recipient.pseudonym_hex != trigger_pseudonym)
            .filter_map(|recipient| pseudonym_from_hex(&recipient.pseudonym_hex))
            .collect::<Vec<_>>(),
        max_cascades(),
    );
    let Some(_cascade_skipped) =
        wait_for_rotation_slot(state, community_id, Some(channel_id), &candidates).await
    else {
        return Ok(());
    };

    let new_generation = current_generation(state, community_id, Some(channel_id)) + 1;
    let mek = MediaEncryptionKey::generate(new_generation);
    distribute_mek(
        state,
        community_id,
        Some(channel_id),
        &mek,
        &recipients,
        true,
    )
    .await?;

    state.channel_mek_cache.lock().insert(
        (community_id.to_string(), channel_id.to_string()),
        mek.clone(),
    );
    update_generation_state(state, community_id, Some(channel_id), new_generation);
    persist_mek(app_handle, community_id, Some(channel_id), &mek);
    emit_rotation_event(app_handle, community_id, Some(channel_id), new_generation);

    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: Some(channel_id.to_string()),
            new_generation,
            rotator_pseudonym: my_pseudonym_hex(state, community_id),
        }),
    )?;
    Ok(())
}

pub async fn handle_request_mek(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    needed_generation: u64,
    requester_pseudonym: &str,
) -> Result<(), String> {
    let recipients = online_recipients(state, community_id, None);
    let candidates = recipients
        .iter()
        .map(|recipient| recipient.pseudonym_hex.clone())
        .collect::<Vec<_>>();
    if !selected_request_responder(state, community_id, requester_pseudonym, &candidates)? {
        return Ok(());
    }

    let requester_route = recipients
        .iter()
        .find(|recipient| recipient.pseudonym_hex == requester_pseudonym)
        .ok_or("requester route not found")?
        .clone();
    let mek = lookup_mek(
        app_handle,
        state,
        community_id,
        channel_id,
        needed_generation,
    )
    .ok_or("needed MEK generation not available")?;
    distribute_mek(
        state,
        community_id,
        Some(channel_id),
        &mek,
        &[requester_route],
        false,
    )
    .await
}

pub fn handle_incoming_mek_transfer(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<u64, String> {
    let secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or("identity secret unavailable")?;
    let mek = unwrap_received_mek(community_id, &secret, sender_pseudonym, wrapped_mek)?;
    let generation = mek.generation();
    apply_received_mek(state, community_id, channel_id, &mek);
    persist_mek(app_handle, community_id, channel_id, &mek);
    emit_rotation_event(app_handle, community_id, channel_id, generation);
    Ok(generation)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::state::{AppState, ChannelInfo, ChannelType, CommunityRecords, CommunityState};
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_crypto::group::mek_distribution::wrap_mek;
    use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

    use super::{apply_received_mek, selected_request_responder, unwrap_received_mek};

    fn pseudo_hex(seed: u8) -> String {
        hex::encode([seed; 32])
    }

    fn state_with_pseudonym(seed: u8) -> Arc<AppState> {
        let state = Arc::new(AppState::default());
        state.communities.write().insert(
            "community".into(),
            CommunityState {
                id: "community".into(),
                name: "Community".into(),
                description: None,
                icon_hash: None,
                banner_hash: None,
                channels: vec![ChannelInfo {
                    id: "voice".into(),
                    name: "Voice".into(),
                    channel_type: ChannelType::Voice,
                    unread_count: 0,
                    category_id: None,
                    topic: String::new(),
                    forum_tags: None,
                    stage_speakers: Vec::new(),
                    stage_moderator: None,
                    slowmode_seconds: None,
                    nsfw: false,
                    message_record_key: None,
                    mek_generation: 0,
                    notification_level: "all".to_string(),
                    notification_sound_ref: None,
                    parent_voice_channel_id: None,
                }],
                categories: Vec::new(),
                my_role_ids: vec![0],
                roles: Vec::new(),
                dht_owner_keypair: None,
                my_pseudonym_key: Some(pseudo_hex(seed)),
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
                my_segment_index: None,
                governance_key: None,
                governance_state: None,
                lamport_counter: 0,
                gossip: None,
                slot_keypair: None,
                channel_log_keys: std::collections::HashMap::new(),
                registry_owner_keypair: None,
                slot_seed: None,
                known_members: std::collections::HashSet::new(),
                member_roles: std::collections::HashMap::new(),
                channel_sequences: std::collections::HashMap::new(),
                pending_syncs: std::collections::HashMap::new(),
                watched_records: std::collections::HashSet::new(),
                record_sequences: std::collections::HashMap::new(),
                peer_sequences: std::collections::HashMap::new(),
                channel_last_send_at: std::collections::HashMap::new(),
                peer_reliability: std::collections::HashMap::new(),
                presence_poll_shutdown_tx: None,
                dht_keepalive_shutdown_tx: None,
                open_community_records: CommunityRecords::default(),
                my_event_rsvps: std::collections::HashMap::new(),
                event_rsvps_by_event: std::collections::HashMap::new(),
                onboarding_complete: false,
                my_bio: None,
                my_pronouns: None,
                my_theme_color: None,
                my_badges: Vec::new(),
                my_avatar_ref: None,
                my_banner_ref: None,
                member_profiles: std::collections::HashMap::new(),
                recent_member_joins: std::collections::VecDeque::new(),
            },
        );
        state
    }

    #[test]
    fn request_mek_only_selected_responder_replies() {
        let requester = pseudo_hex(9);
        let candidates = vec![
            requester.clone(),
            pseudo_hex(1),
            pseudo_hex(2),
            pseudo_hex(3),
        ];

        let selected_states = [1_u8, 2, 3]
            .into_iter()
            .filter(|seed| {
                let state = state_with_pseudonym(*seed);
                selected_request_responder(&state, "community", &requester, &candidates)
                    .expect("selection should succeed")
            })
            .collect::<Vec<_>>();

        assert_eq!(selected_states.len(), 1, "only one responder should reply");
    }

    #[test]
    fn request_mek_requester_never_self_selects() {
        let requester = pseudo_hex(7);
        let state = state_with_pseudonym(7);
        let candidates = vec![requester.clone(), pseudo_hex(8)];

        assert!(
            !selected_request_responder(&state, "community", &requester, &candidates)
                .expect("selection should succeed")
        );
    }

    #[test]
    fn incoming_mek_updates_channel_cache_and_generation() {
        let state = state_with_pseudonym(1);
        let mek = MediaEncryptionKey::generate(11);

        apply_received_mek(&state, "community", Some("voice"), &mek);

        let cached = state
            .channel_mek_cache
            .lock()
            .get(&("community".to_string(), "voice".to_string()))
            .cloned()
            .expect("channel MEK should be cached");
        assert_eq!(cached.generation(), 11);

        let generation = state
            .communities
            .read()
            .get("community")
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == "voice")
            })
            .map(|channel| channel.mek_generation)
            .expect("voice channel should exist");
        assert_eq!(generation, 11);
    }

    #[test]
    fn incoming_mek_updates_community_cache_and_generation() {
        let state = state_with_pseudonym(1);
        let mek = MediaEncryptionKey::generate(5);

        apply_received_mek(&state, "community", None, &mek);

        let cached = state
            .mek_cache
            .lock()
            .get("community")
            .cloned()
            .expect("community MEK should be cached");
        assert_eq!(cached.generation(), 5);

        let generation = state
            .communities
            .read()
            .get("community")
            .map(|community| community.mek_generation)
            .expect("community should exist");
        assert_eq!(generation, 5);
    }

    #[test]
    fn incoming_mek_transfer_unwraps_real_wrapped_payload() {
        let sender_secret = [2_u8; 32];
        let recipient_secret = [1_u8; 32];
        let sender_key = derive_community_pseudonym(&sender_secret, "community");
        let recipient_key = derive_community_pseudonym(&recipient_secret, "community");
        let mek = MediaEncryptionKey::generate(17);
        let wrapped = wrap_mek(
            &sender_key,
            &recipient_key.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .expect("wrap should succeed");

        let unwrapped = unwrap_received_mek(
            "community",
            &recipient_secret,
            &hex::encode(sender_key.verifying_key().to_bytes()),
            &wrapped,
        )
        .expect("unwrap should succeed");

        assert_eq!(unwrapped.generation(), 17);
        assert_eq!(unwrapped.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn incoming_mek_transfer_rejects_wrong_sender_identity() {
        let sender_secret = [2_u8; 32];
        let recipient_secret = [1_u8; 32];
        let fake_sender_secret = [3_u8; 32];
        let sender_key = derive_community_pseudonym(&sender_secret, "community");
        let recipient_key = derive_community_pseudonym(&recipient_secret, "community");
        let fake_sender_key = derive_community_pseudonym(&fake_sender_secret, "community");
        let mek = MediaEncryptionKey::generate(17);
        let wrapped = wrap_mek(
            &sender_key,
            &recipient_key.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .expect("wrap should succeed");

        let result = unwrap_received_mek(
            "community",
            &recipient_secret,
            &hex::encode(fake_sender_key.verifying_key().to_bytes()),
            &wrapped,
        );

        assert!(result.is_err(), "wrong sender identity must not unwrap");
    }
}
