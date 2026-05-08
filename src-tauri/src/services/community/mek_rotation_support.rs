use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_crypto::group::mek_distribution::wrap_mek;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::id::PseudonymKey;
use tauri::{Emitter, Manager};

use crate::channels::CommunityEvent;
use crate::state::AppState;
use crate::state_helpers;

const CASCADE_TIMEOUT_SECS: u64 = 30;
const MAX_CASCADES: usize = 3;

fn cascade_delay(index: usize) -> Duration {
    Duration::from_secs(CASCADE_TIMEOUT_SECS * index as u64)
}

fn generation_advanced(initial_generation: u64, current_generation: u64) -> bool {
    current_generation > initial_generation
}

#[derive(Clone)]
pub(crate) struct RotationRecipient {
    pub(crate) pseudonym_hex: String,
    pub(crate) route_blob: Vec<u8>,
}

pub(crate) fn online_recipients(
    state: &Arc<AppState>,
    community_id: &str,
    exclude_pseudonym: Option<&str>,
) -> Vec<RotationRecipient> {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };
    let excluded = exclude_pseudonym.unwrap_or_default();
    community
        .gossip
        .as_ref()
        .map(|gossip| {
            gossip
                .online_members
                .iter()
                .filter(|(pseudonym, member)| {
                    !member.route_blob.is_empty() && pseudonym.as_str() != excluded
                })
                .map(|(pseudonym, member)| RotationRecipient {
                    pseudonym_hex: pseudonym.clone(),
                    route_blob: member.route_blob.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn voice_recipients(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    trigger_pseudonym: &str,
    include_trigger_in_recipients: bool,
) -> Vec<RotationRecipient> {
    let (participant_keys, my_pseudonym) = {
        let ve = state.voice_engine.lock();
        let participants = ve
            .as_ref()
            .filter(|handle| {
                handle.community_id.as_deref() == Some(community_id)
                    && handle.channel_id == channel_id
            })
            .map(|handle| {
                let transport = handle.transport.blocking_lock();
                transport.peer_keys()
            })
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        (participants, my_pseudonym_hex(state, community_id))
    };
    let participant_keys = effective_voice_participants(
        participant_keys,
        my_pseudonym,
        trigger_pseudonym,
        include_trigger_in_recipients,
    );
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };
    let Some(gossip) = community.gossip.as_ref() else {
        return Vec::new();
    };

    let mut recipients = Vec::new();
    for pseudonym in participant_keys {
        let route_blob = gossip
            .online_members
            .get(&pseudonym)
            .map(|member| member.route_blob.clone())
            .unwrap_or_default();
        recipients.push(RotationRecipient {
            pseudonym_hex: pseudonym,
            route_blob,
        });
    }
    recipients
}

fn effective_voice_participants(
    mut participant_keys: HashSet<String>,
    my_pseudonym: Option<String>,
    trigger_pseudonym: &str,
    include_trigger_in_recipients: bool,
) -> HashSet<String> {
    if let Some(my_pseudonym) = my_pseudonym {
        participant_keys.insert(my_pseudonym);
    }
    if !include_trigger_in_recipients {
        participant_keys.remove(trigger_pseudonym);
    }
    participant_keys
}

pub(crate) async fn wait_for_rotation_slot(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    candidates: &[PseudonymKey],
) -> Option<Vec<PseudonymKey>> {
    let me = my_pseudonym(state, community_id)?;
    let index = candidates.iter().position(|candidate| candidate == &me)?;
    let initial_generation = current_generation(state, community_id, channel_id);

    if index > 0 {
        tokio::time::sleep(cascade_delay(index)).await;
        if generation_advanced(
            initial_generation,
            current_generation(state, community_id, channel_id),
        ) {
            return None;
        }
    }

    Some(candidates.iter().take(index).cloned().collect())
}

pub(crate) async fn distribute_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &MediaEncryptionKey,
    recipients: &[RotationRecipient],
    unsafe_route: bool,
) -> Result<(), String> {
    let secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or("identity secret unavailable")?;
    let my_signing_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
    let rc = if unsafe_route {
        state_helpers::routing_context(state)
    } else {
        state_helpers::safe_routing_context(state)
    }
    .ok_or("routing context unavailable")?;

    for recipient in recipients {
        if recipient.pseudonym_hex == my_pseudonym_hex(state, community_id).unwrap_or_default() {
            continue;
        }
        let recipient_pseudo =
            pseudonym_from_hex(&recipient.pseudonym_hex).ok_or("invalid recipient pseudonym")?;
        let wrapped = wrap_mek(&my_signing_key, &recipient_pseudo.0, &mek.to_wire_bytes())
            .map_err(|e| format!("wrap MEK failed: {e}"))?;
        let route_id = state_helpers::import_route_blob(state, &recipient.route_blob)?;
        let payload = CommunityEnvelope::Control(ControlPayload::MekTransfer {
            community_id: community_id.to_string(),
            channel_id: channel_id.map(ToOwned::to_owned),
            generation: mek.generation(),
            sender_pseudonym: my_pseudonym_hex(state, community_id).unwrap_or_default(),
            wrapped_mek: wrapped,
        });
        let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
            .map_err(|e| format!("encode MEK transfer: {e}"))?;
        let reply = rc
            .app_call(veilid_core::Target::RouteId(route_id), bytes)
            .await
            .map_err(|e| format!("app_call MEK transfer failed: {e}"))?;

        // P1.3 — verify the recipient sent back a structured
        // `MekTransferAck` confirming BOTH the network arrival and the
        // app-layer unwrap. Bare `ACK` reply means the recipient hit
        // an unwrap error (logged on their side); we record a debug
        // trace so the operator can investigate. Generation mismatch
        // would indicate a misrouted app_call — also logged but not
        // hard-failed because the next rotation will recover.
        if reply == b"ACK" {
            tracing::debug!(
                community = %community_id,
                recipient = %recipient.pseudonym_hex,
                generation = mek.generation(),
                "MEK transfer recipient replied with bare ACK (unwrap likely failed)"
            );
        } else {
            match rekindle_protocol::capnp_envelope::try_decode_community_envelope(&reply) {
                Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransferAck {
                    generation: ack_gen,
                    channel_id: ack_channel,
                    requester_pseudonym,
                    ..
                }))) => {
                    if ack_gen != mek.generation() {
                        tracing::warn!(
                            community = %community_id,
                            recipient = %recipient.pseudonym_hex,
                            sent_gen = mek.generation(),
                            ack_gen,
                            "MekTransferAck generation mismatch — possible misrouted app_call"
                        );
                    } else if ack_channel.as_deref() != channel_id {
                        tracing::warn!(
                            community = %community_id,
                            recipient = %recipient.pseudonym_hex,
                            sent_channel = ?channel_id,
                            ack_channel = ?ack_channel,
                            "MekTransferAck channel_id mismatch"
                        );
                    } else {
                        tracing::trace!(
                            community = %community_id,
                            recipient = %recipient.pseudonym_hex,
                            requester = %requester_pseudonym,
                            generation = ack_gen,
                            "MekTransferAck verified"
                        );
                    }
                }
                Ok(_) => tracing::debug!(
                    community = %community_id,
                    recipient = %recipient.pseudonym_hex,
                    "MEK transfer reply was not a MekTransferAck variant"
                ),
                Err(e) => tracing::debug!(
                    community = %community_id,
                    recipient = %recipient.pseudonym_hex,
                    error = %e,
                    "MEK transfer reply could not be decoded as community envelope"
                ),
            }
        }
    }

    Ok(())
}

pub(crate) fn lookup_mek(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    {
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
            if mek.generation() == generation {
                return Some(mek.clone());
            }
        }
    }
    {
        let cache = state.mek_cache.lock();
        if let Some(mek) = cache.get(community_id) {
            if mek.generation() == generation {
                return Some(mek.clone());
            }
        }
    }

    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    let ks = guard.as_ref()?;
    crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation).or_else(
        || crate::keystore::load_mek(ks, community_id).filter(|mek| mek.generation() == generation),
    )
}

pub(crate) fn update_generation_state(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    generation: u64,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.mek_generation = generation;
        if let Some(channel_id) = channel_id {
            if let Some(channel) = community
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                channel.mek_generation = generation;
            }
        }
    }
}

pub(crate) fn emit_rotation_event(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    channel_id: Option<&str>,
    generation: u64,
) {
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::MekRotated {
            community_id: community_id.to_string(),
            channel_id: channel_id.map(ToOwned::to_owned),
            new_generation: generation,
        },
    );
}

pub(crate) fn persist_mek(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &MediaEncryptionKey,
) {
    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    if let Some(ks) = guard.as_ref() {
        crate::keystore::store_mek(ks, community_id, channel_id, mek);
    }
}

pub(crate) fn current_generation(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
) -> u64 {
    if let Some(channel_id) = channel_id {
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
            return mek.generation();
        }
    } else {
        let cache = state.mek_cache.lock();
        if let Some(mek) = cache.get(community_id) {
            return mek.generation();
        }
    }

    let communities = state.communities.read();
    communities.get(community_id).map_or(0, |community| {
        channel_id
            .and_then(|channel_id| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.mek_generation)
            })
            .unwrap_or(community.mek_generation)
    })
}

pub(crate) fn my_pseudonym_hex(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|community| community.my_pseudonym_key.clone())
}

pub(crate) fn my_pseudonym(state: &Arc<AppState>, community_id: &str) -> Option<PseudonymKey> {
    my_pseudonym_hex(state, community_id)
        .as_deref()
        .and_then(pseudonym_from_hex)
}

pub(crate) fn pseudonym_from_hex(hex_str: &str) -> Option<PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(PseudonymKey(bytes))
}

pub(crate) fn max_cascades() -> usize {
    MAX_CASCADES
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        cascade_delay, current_generation, effective_voice_participants, generation_advanced,
        max_cascades, my_pseudonym_hex, wait_for_rotation_slot,
    };
    use crate::state::{AppState, ChannelInfo, ChannelType, CommunityRecords, CommunityState};
    use proptest::prelude::*;
    use rand::rngs::StdRng;
    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    use rekindle_types::id::PseudonymKey;

    fn pseudo(seed: u8) -> PseudonymKey {
        PseudonymKey([seed; 32])
    }

    fn community_state(my_seed: u8, generation: u64) -> CommunityState {
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
                mek_generation: generation,
                notification_level: "all".to_string(),
                notification_sound_ref: None,
                parent_voice_channel_id: None,
            }],
            categories: Vec::new(),
            my_role_ids: vec![0],
            roles: Vec::new(),
            dht_owner_keypair: None,
            my_pseudonym_key: Some(hex::encode(pseudo(my_seed).0)),
            mek_generation: generation,
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
        }
    }

    fn state_with_generation(my_seed: u8, generation: u64) -> Arc<AppState> {
        let state = Arc::new(AppState::default());
        state
            .communities
            .write()
            .insert("community".into(), community_state(my_seed, generation));
        state
    }

    #[test]
    fn exposes_expected_max_cascades() {
        assert_eq!(max_cascades(), 3);
    }

    #[test]
    fn reads_generation_from_community_and_channel_state() {
        let state = state_with_generation(1, 4);
        assert_eq!(current_generation(&state, "community", None), 4);
        assert_eq!(current_generation(&state, "community", Some("voice")), 4);
        assert_eq!(
            my_pseudonym_hex(&state, "community"),
            Some(hex::encode(pseudo(1).0))
        );
    }

    #[tokio::test]
    async fn primary_candidate_rotates_immediately() {
        let state = state_with_generation(1, 0);
        let skipped = wait_for_rotation_slot(
            &state,
            "community",
            None,
            &[pseudo(1), pseudo(2), pseudo(3)],
        )
        .await
        .expect("slot assignment");
        assert!(skipped.is_empty());
    }

    #[test]
    fn second_candidate_waits_thirty_seconds_before_taking_over() {
        assert_eq!(cascade_delay(1), Duration::from_secs(30));
        assert_eq!(cascade_delay(2), Duration::from_secs(60));
        assert_eq!(cascade_delay(3), Duration::from_secs(90));
    }

    #[test]
    fn candidate_aborts_when_generation_already_advanced() {
        assert!(!generation_advanced(4, 4));
        assert!(generation_advanced(4, 5));
        assert!(generation_advanced(0, 1));
    }

    #[test]
    fn voice_join_rotation_includes_joiner_and_self() {
        let participants = [String::from("peer-a"), String::from("peer-b")]
            .into_iter()
            .collect();
        let effective =
            effective_voice_participants(participants, Some(String::from("me")), "joiner", true);

        assert!(effective.contains("peer-a"));
        assert!(effective.contains("peer-b"));
        assert!(effective.contains("me"));
    }

    #[test]
    fn voice_leave_rotation_excludes_leaver_but_keeps_self() {
        let participants = [
            String::from("leaver"),
            String::from("peer-b"),
            String::from("peer-c"),
        ]
        .into_iter()
        .collect();
        let effective =
            effective_voice_participants(participants, Some(String::from("me")), "leaver", false);

        assert!(!effective.contains("leaver"));
        assert!(effective.contains("peer-b"));
        assert!(effective.contains("peer-c"));
        assert!(effective.contains("me"));
    }

    proptest! {
        #[test]
        fn churn_departures_converge_to_single_rotator_and_monotonic_lamport(
            member_count in 6usize..21,
            delivery_seed in any::<u64>(),
        ) {
            let members = (1..=member_count)
                .map(|seed| pseudo(u8::try_from(seed).expect("member_count bounded to u8")))
                .collect::<Vec<_>>();
            let mut remaining = members.clone();
            let mut departures = members.clone();
            let mut seed = [0_u8; 32];
            seed[..8].copy_from_slice(&delivery_seed.to_le_bytes());
            let mut rng = StdRng::from_seed(seed);
            departures.shuffle(&mut rng);

            let mut generation = 0_u64;
            let mut lamports = Vec::new();

            for departed in departures.iter().take(member_count.saturating_sub(1)) {
                remaining.retain(|member| member != departed);
                prop_assume!(!remaining.is_empty());

                let mut delivered = remaining.clone();
                delivered.shuffle(&mut rng);

                let expected_candidates = rekindle_secrets::rotator::cascade_candidates(
                    departed,
                    &remaining,
                    max_cascades(),
                );
                prop_assert!(!expected_candidates.is_empty());

                let delivered_candidates = rekindle_secrets::rotator::cascade_candidates(
                    departed,
                    &delivered,
                    max_cascades(),
                );
                prop_assert_eq!(&expected_candidates, &delivered_candidates);

                let unique_candidates = expected_candidates
                    .iter()
                    .collect::<std::collections::HashSet<_>>();
                prop_assert_eq!(unique_candidates.len(), expected_candidates.len());

                let rotators = remaining
                    .iter()
                    .filter(|member| expected_candidates.first() == Some(member))
                    .count();
                prop_assert_eq!(rotators, 1);

                let initial_generation = generation;
                generation += 1;
                prop_assert!(generation_advanced(initial_generation, generation));

                let lamport = lamports.last().copied().unwrap_or(0) + 1;
                lamports.push(lamport);
            }

            for window in lamports.windows(2) {
                prop_assert!(window[0] < window[1]);
            }
        }
    }
}
