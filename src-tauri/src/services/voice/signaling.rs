//! Voice signaling handlers for community gossip payloads.
//!
//! Grouped separately from the Veilid dispatch modules so voice gossip handling
//! stays local to the voice service.

use std::sync::Arc;

use tauri::Emitter;

use crate::channels::CommunityEvent;
use crate::state::AppState;
use rekindle_protocol::dht::community::envelope::ControlPayload;

/// Handle voice-related control payloads received via gossip.
pub(crate) fn handle_voice_signaling(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
) {
    match payload {
        ControlPayload::VoiceJoin {
            channel_id,
            route_blob,
        } => {
            handle_voice_join(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                channel_id,
                route_blob,
            );
        }
        ControlPayload::VoiceLeave { channel_id } => {
            handle_voice_leave(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                channel_id,
            );
        }
        ControlPayload::VoiceModeSwitch {
            channel_id,
            mode,
            host_pseudonym,
        } => {
            let _ = app_handle.emit(
                "community-event",
                CommunityEvent::VoiceModeSwitch {
                    community_id: community_id.to_string(),
                    channel_id,
                    mode,
                    host_pseudonym,
                },
            );
        }
        ControlPayload::StageUpdate {
            channel_id,
            topic,
            speakers,
            moderator_pseudonym,
            lamport: _,
        } => {
            handle_stage_update(
                app_handle,
                state,
                community_id,
                channel_id,
                topic.as_ref(),
                &speakers,
                &moderator_pseudonym,
            );
        }
        ControlPayload::SpeakRequest {
            channel_id,
            requester_pseudonym,
            lamport: _,
        } => {
            handle_speak_request(
                app_handle,
                state,
                community_id,
                channel_id,
                requester_pseudonym,
            );
        }
        ControlPayload::SpeakResponse {
            channel_id,
            requester_pseudonym,
            granted,
            moderator_pseudonym,
            lamport: _,
        } => {
            handle_speak_response(
                app_handle,
                state,
                community_id,
                channel_id,
                requester_pseudonym,
                granted,
                moderator_pseudonym,
            );
        }
        ControlPayload::VoiceMute {
            channel_id: _,
            target_pseudonym,
            muted,
        } => {
            handle_voice_mute(app_handle, state, community_id, &target_pseudonym, muted);
        }
        ControlPayload::VoiceDeafen {
            channel_id: _,
            target_pseudonym,
            deafened,
        } => {
            handle_voice_deafen(state, community_id, &target_pseudonym, deafened);
        }
        ControlPayload::VoiceRoster {
            channel_id: _,
            participants,
        } => {
            handle_voice_roster(state, participants);
        }
        ControlPayload::SoundboardPlay {
            channel_id,
            expression_id,
            actor_pseudonym,
        } => {
            // Architecture §9.3 reader-validates + §10.9 — only play if
            // the gossip-verified sender matches the claimed actor AND
            // they hold USE_SOUNDBOARD. Otherwise any community member
            // could blast audio claiming to be anyone.
            if !actor_pseudonym.eq_ignore_ascii_case(sender_pseudonym) {
                tracing::debug!(
                    community = %community_id,
                    sender = %sender_pseudonym,
                    actor = %actor_pseudonym,
                    "dropping SoundboardPlay: actor_pseudonym mismatch",
                );
                return;
            }
            if !sender_has_soundboard_perm(state, community_id, sender_pseudonym) {
                tracing::debug!(
                    community = %community_id,
                    sender = %sender_pseudonym,
                    "dropping SoundboardPlay: sender lacks USE_SOUNDBOARD",
                );
                return;
            }
            handle_soundboard_play(
                app_handle,
                community_id,
                &channel_id,
                &expression_id,
                &actor_pseudonym,
            );
        }
        _ => {}
    }
}

/// Architecture §9.3 — verify the gossip sender currently holds the
/// USE_SOUNDBOARD permission in the community's merged governance.
/// Returns false on missing state, bad pseudonym hex, or insufficient
/// permission.
fn sender_has_soundboard_perm(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym_hex: &str,
) -> bool {
    use rekindle_governance::permissions::compute_permissions;
    use rekindle_types::id::PseudonymKey;
    use rekindle_types::permissions::USE_SOUNDBOARD;
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return false;
    };
    let Some(gov) = community.governance_state.as_ref() else {
        return false;
    };
    let Ok(bytes) = hex::decode(sender_pseudonym_hex) else {
        return false;
    };
    let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) else {
        return false;
    };
    let perms = compute_permissions(
        &PseudonymKey(arr),
        None,
        gov,
        rekindle_utils::timestamp_secs(),
    );
    perms & USE_SOUNDBOARD == USE_SOUNDBOARD
}

/// Architecture §10.9: relay the soundboard trigger to the frontend so
/// the audio plays locally. The bytes already live in the local
/// `expressions` cache; the gossip envelope only carries the ID.
fn handle_soundboard_play(
    app_handle: &tauri::AppHandle,
    community_id: &str,
    channel_id: &str,
    expression_id: &str,
    actor_pseudonym: &str,
) {
    use tauri::Emitter as _;
    let _ = app_handle.emit(
        "community-event",
        crate::channels::CommunityEvent::SoundboardPlay {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            expression_id: expression_id.to_string(),
            actor_pseudonym: actor_pseudonym.to_string(),
        },
    );
}

fn handle_voice_join(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    route_blob: Vec<u8>,
) {
    if !is_stage_channel(state, community_id, &channel_id) {
        let state = state.clone();
        let app_handle = app_handle.clone();
        let community_id = community_id.to_string();
        let channel_id_clone = channel_id.clone();
        let sender = sender_pseudonym.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::services::community::rotate_voice_mek_for_membership(
                &app_handle,
                &state,
                &community_id,
                &channel_id_clone,
                &sender,
                true,
            )
            .await
            {
                tracing::debug!(community = %community_id, channel = %channel_id_clone, error = %error, "voice join MEK rotation skipped");
            }
        });
    }
    let (shared_transport, my_pseudonym) = {
        let ve = state.voice_engine.lock();
        let transport = ve.as_ref().map(|h| h.transport.clone());
        drop(ve);
        let pk = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .and_then(|c| c.my_pseudonym_key.clone())
                .unwrap_or_default()
        };
        (transport, pk)
    };
    if let Some(ref transport) = shared_transport {
        let sender_key = sender_pseudonym.to_string();
        let blob = route_blob.clone();
        let transport = transport.clone();
        let my_pk = my_pseudonym.clone();
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let is_stage = is_stage_channel(&state, &cid, &ch_id);
            let (peer_count, current_mode) = {
                let mut t = transport.lock().await;
                if let Err(e) = t.add_peer(&sender_key, &blob) {
                    tracing::warn!(peer = %sender_key, error = %e, "failed to add voice peer");
                }
                (t.peer_count(), t.mode().clone())
            };

            if is_stage {
                reconcile_stage_transport(&state, &cid, &ch_id, &transport, &my_pk).await;
                return;
            }

            // Architecture §10.2 lines 2017-2019: ≤4 members full-mesh,
            // >4 members SFU. Switch when the 5th member joins. peer_count
            // excludes self, so peer_count >= 4 means total >= 5.
            if peer_count >= 4 && matches!(current_mode, rekindle_voice::VoiceMode::Mesh) {
                let mut candidates = {
                    let t = transport.lock().await;
                    t.peer_keys()
                };
                candidates.push(my_pk.clone());
                let target = crate::services::voice::election::channel_target(&ch_id);
                let Some(elected_host) =
                    crate::services::voice::election::elect_relay_host(candidates.iter(), &target)
                else {
                    tracing::warn!(channel = %ch_id, "MCU election skipped — no decodable candidates");
                    return;
                };

                tracing::info!(
                    peer_count = peer_count + 1,
                    host = %elected_host,
                    "auto-electing MCU host (5+ participants)"
                );

                let mode_envelope =
                    rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                        ControlPayload::VoiceModeSwitch {
                            channel_id: ch_id.clone(),
                            mode: "mcu".to_string(),
                            host_pseudonym: Some(elected_host.clone()),
                        },
                    );
                let _ = crate::services::community::send_to_mesh(&state, &cid, &mode_envelope);

                {
                    let mut t = transport.lock().await;
                    t.set_mode(rekindle_voice::VoiceMode::Mcu {
                        host_pseudonym: elected_host.clone(),
                    });
                }

                if elected_host == my_pk {
                    let _ = crate::services::voice::session::start_mcu_loop(&state);
                }
            }

            // Broadcast voice roster so the new joiner discovers all current participants
            let participants: Vec<rekindle_protocol::dht::community::envelope::VoiceRosterEntry> = {
                let t = transport.lock().await;
                let communities = state.communities.read();
                let cs = communities.get(&cid);
                t.peer_keys()
                    .iter()
                    .filter_map(|pk| {
                        let gossip = cs?.gossip.as_ref()?;
                        let member = gossip.online_members.get(pk)?;
                        Some(
                            rekindle_protocol::dht::community::envelope::VoiceRosterEntry {
                                pseudonym_key: pk.clone(),
                                route_blob: member.route_blob.clone(),
                                muted: false,
                                deafened: false,
                            },
                        )
                    })
                    .collect()
            };
            if !participants.is_empty() {
                let roster_envelope =
                    rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                        ControlPayload::VoiceRoster {
                            channel_id: ch_id,
                            participants,
                        },
                    );
                let _ = crate::services::community::send_to_mesh(&state, &cid, &roster_envelope);
            }
        });
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::VoiceJoin {
            community_id: community_id.to_string(),
            channel_id,
            pseudonym_key: sender_pseudonym.to_string(),
            route_blob,
        },
    );
}

fn handle_voice_leave(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
) {
    if !is_stage_channel(state, community_id, &channel_id) {
        let state = state.clone();
        let app_handle = app_handle.clone();
        let community_id = community_id.to_string();
        let channel_id_clone = channel_id.clone();
        let sender = sender_pseudonym.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::services::community::rotate_voice_mek_for_membership(
                &app_handle,
                &state,
                &community_id,
                &channel_id_clone,
                &sender,
                false,
            )
            .await
            {
                tracing::debug!(community = %community_id, channel = %channel_id_clone, error = %error, "voice leave MEK rotation skipped");
            }
        });
    }
    let (shared_transport, my_pseudonym) = {
        let ve = state.voice_engine.lock();
        let transport = ve.as_ref().map(|h| h.transport.clone());
        drop(ve);
        let pk = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .and_then(|c| c.my_pseudonym_key.clone())
                .unwrap_or_default()
        };
        (transport, pk)
    };
    if let Some(ref transport) = shared_transport {
        let sender_key = sender_pseudonym.to_string();
        let transport = transport.clone();
        let my_pk = my_pseudonym.clone();
        let cid = community_id.to_string();
        let ch_id = channel_id.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let is_stage = is_stage_channel(&state, &cid, &ch_id);
            let (peer_count, current_mode) = {
                let mut t = transport.lock().await;
                t.remove_peer(&sender_key);
                (t.peer_count(), t.mode().clone())
            };

            if is_stage {
                reconcile_stage_transport(&state, &cid, &ch_id, &transport, &my_pk).await;
                return;
            }

            if let rekindle_voice::VoiceMode::Mcu { ref host_pseudonym } = current_mode {
                if *host_pseudonym == sender_key {
                    // MCU host left — fall back to mesh
                    tracing::info!("MCU host left — falling back to mesh");
                    {
                        let mut t = transport.lock().await;
                        t.set_mode(rekindle_voice::VoiceMode::Mesh);
                    }
                    crate::services::voice::session::stop_mcu_loop(&state).await;

                    let mesh_envelope =
                        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                            ControlPayload::VoiceModeSwitch {
                                channel_id: ch_id.clone(),
                                mode: "mesh".to_string(),
                                host_pseudonym: None,
                            },
                        );
                    let _ = crate::services::community::send_to_mesh(&state, &cid, &mesh_envelope);

                    // Re-elect if total ≥ 5 participants (peer_count + self).
                    // Architecture §10.2 line 2017: >4 members → SFU.
                    if peer_count >= 4 {
                        let mut candidates = {
                            let t = transport.lock().await;
                            t.peer_keys()
                        };
                        candidates.push(my_pk.clone());
                        let target = crate::services::voice::election::channel_target(&ch_id);
                        let Some(elected_host) =
                            crate::services::voice::election::elect_relay_host(
                                candidates.iter(),
                                &target,
                            )
                        else {
                            tracing::warn!(
                                channel = %ch_id,
                                "MCU re-election skipped — no decodable candidates",
                            );
                            return;
                        };

                        let mode_envelope =
                            rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                                ControlPayload::VoiceModeSwitch {
                                    channel_id: ch_id,
                                    mode: "mcu".to_string(),
                                    host_pseudonym: Some(elected_host.clone()),
                                },
                            );
                        let _ =
                            crate::services::community::send_to_mesh(&state, &cid, &mode_envelope);

                        {
                            let mut t = transport.lock().await;
                            t.set_mode(rekindle_voice::VoiceMode::Mcu {
                                host_pseudonym: elected_host.clone(),
                            });
                        }
                        if elected_host == my_pk {
                            let _ = crate::services::voice::session::start_mcu_loop(&state);
                        }
                    }
                } else if peer_count < 4 {
                    // Total ≤ 4 — full-mesh per architecture §10.2 line 2017.
                    tracing::info!(peer_count, "below MCU threshold — switching to mesh");
                    {
                        let mut t = transport.lock().await;
                        t.set_mode(rekindle_voice::VoiceMode::Mesh);
                    }
                    crate::services::voice::session::stop_mcu_loop(&state).await;

                    let mesh_envelope =
                        rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                            ControlPayload::VoiceModeSwitch {
                                channel_id: ch_id,
                                mode: "mesh".to_string(),
                                host_pseudonym: None,
                            },
                        );
                    let _ = crate::services::community::send_to_mesh(&state, &cid, &mesh_envelope);
                }
            }
        });
    }
    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::VoiceLeave {
            community_id: community_id.to_string(),
            channel_id,
            pseudonym_key: sender_pseudonym.to_string(),
        },
    );
}

fn handle_voice_mute(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    target_pseudonym: &str,
    muted: bool,
) {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_muted(muted);
            handle
                .muted_flag
                .store(muted, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let _ = app_handle.emit(
        "voice-event",
        crate::channels::VoiceEvent::UserMuted {
            public_key: target_pseudonym.to_string(),
            muted,
        },
    );
}

fn handle_voice_deafen(
    state: &Arc<AppState>,
    community_id: &str,
    target_pseudonym: &str,
    deafened: bool,
) {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_deafened(deafened);
            handle
                .deafened_flag
                .store(deafened, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn handle_voice_roster(
    state: &Arc<AppState>,
    participants: Vec<rekindle_protocol::dht::community::envelope::VoiceRosterEntry>,
) {
    let ve = state.voice_engine.lock();
    if let Some(ref handle) = *ve {
        let transport = handle.transport.clone();
        tokio::spawn(async move {
            let mut t = transport.lock().await;
            for entry in participants {
                if !entry.route_blob.is_empty() {
                    let _ = t.add_peer(&entry.pseudonym_key, &entry.route_blob);
                }
            }
        });
    }
}

fn handle_stage_update(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: String,
    topic: Option<&String>,
    speakers: &[String],
    moderator_pseudonym: &str,
) {
    let community_id_owned = community_id.to_string();
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id_owned) {
            if let Some(channel) = community.channels.iter_mut().find(|channel| channel.id == channel_id) {
                if let Some(topic_value) = topic {
                    channel.topic.clone_from(topic_value);
                }
                channel.stage_speakers.clone_from(&speakers.to_vec());
                channel.stage_moderator = Some(moderator_pseudonym.to_string());
            }
        }
    }

    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::StageUpdate {
            community_id: community_id_owned.clone(),
            channel_id: channel_id.clone(),
            topic: topic.cloned(),
            speakers: speakers.to_vec(),
            moderator_pseudonym: moderator_pseudonym.to_string(),
        },
    );

    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        let my_pseudonym = my_pseudonym_for(&state, &community_id_owned).unwrap_or_default();
        let transport = {
            let ve = state.voice_engine.lock();
            ve.as_ref().map(|handle| handle.transport.clone())
        };
        if let Some(transport) = transport {
            reconcile_stage_transport(
                &state,
                &community_id_owned,
                &channel_id,
                &transport,
                &my_pseudonym,
            )
                .await;
        }
    });
}

fn handle_speak_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: String,
    requester_pseudonym: String,
) {
    let Some(channel_id_bytes) = decode_channel_id(&channel_id) else {
        return;
    };
    let perms = crate::state_helpers::my_permissions(
        state,
        community_id,
        Some(&rekindle_types::id::ChannelId(channel_id_bytes)),
    );
    if perms & rekindle_types::permissions::MANAGE_MESSAGES == 0
        && perms & rekindle_types::permissions::ADMINISTRATOR == 0
    {
        return;
    }

    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::SpeakRequest {
            community_id: community_id.to_string(),
            channel_id,
            requester_pseudonym,
        },
    );
}

fn handle_speak_response(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: String,
    requester_pseudonym: String,
    granted: bool,
    moderator_pseudonym: String,
) {
    let my_pseudonym = my_pseudonym_for(state, community_id);
    if my_pseudonym.as_deref() == Some(requester_pseudonym.as_str()) {
        let stage_state = state.clone();
        let community_id_owned = community_id.to_string();
        let channel_id_owned = channel_id.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::services::community::persist_hand_raise(
                &stage_state,
                &community_id_owned,
                &channel_id_owned,
                false,
            )
            .await
            {
                tracing::debug!(
                    community = %community_id_owned,
                    channel = %channel_id_owned,
                    error = %error,
                    "failed to clear local hand raise after speak response"
                );
            }
        });

        if granted {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                handle.engine.set_muted(false);
                handle
                    .muted_flag
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    let _ = app_handle.emit(
        "community-event",
        CommunityEvent::SpeakResponse {
            community_id: community_id.to_string(),
            channel_id,
            requester_pseudonym,
            granted,
            moderator_pseudonym,
        },
    );
}

fn is_stage_channel(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> bool {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|community| {
            community
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)
                .map(|channel| matches!(channel.channel_type, crate::state::ChannelType::Stage))
        })
        .unwrap_or(false)
}

fn my_pseudonym_for(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|community| community.my_pseudonym_key.clone())
}

fn decode_channel_id(channel_id: &str) -> Option<[u8; 16]> {
    hex::decode(channel_id).ok()?.try_into().ok()
}

fn stage_speakers_for(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> (Vec<String>, Option<String>) {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|community| {
            community
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)
                .map(|channel| (channel.stage_speakers.clone(), channel.stage_moderator.clone()))
        })
        .unwrap_or_default()
}

fn stage_host_score(channel_id: &str, speaker: &str) -> Option<[u8; 32]> {
    let speaker_bytes: [u8; 32] = hex::decode(speaker).ok()?.try_into().ok()?;
    let channel_hash = blake3::hash(channel_id.as_bytes());
    let mut score = [0u8; 32];
    for (index, byte) in score.iter_mut().enumerate() {
        *byte = speaker_bytes[index] ^ channel_hash.as_bytes()[index];
    }
    Some(score)
}

fn select_stage_host(channel_id: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .filter_map(|candidate| stage_host_score(channel_id, candidate).map(|score| (candidate, score)))
        .min_by(|(_, left), (_, right)| left.cmp(right))
        .map(|(candidate, _)| candidate.clone())
}

async fn reconcile_stage_transport(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    transport: &std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    my_pseudonym: &str,
) {
    let (speakers, _) = stage_speakers_for(state, community_id, channel_id);
    let mut candidates = {
        let transport = transport.lock().await;
        transport.peer_keys()
    };
    if !my_pseudonym.is_empty() {
        candidates.push(my_pseudonym.to_string());
    }

    let mut stage_candidates: Vec<String> = candidates
        .iter()
        .filter(|candidate| speakers.contains(candidate))
        .cloned()
        .collect();
    if stage_candidates.is_empty() {
        stage_candidates = candidates;
    }
    stage_candidates.sort();
    stage_candidates.dedup();

    let Some(host_pseudonym) = select_stage_host(channel_id, &stage_candidates) else {
        return;
    };

    {
        let mut transport = transport.lock().await;
        transport.set_mode(rekindle_voice::VoiceMode::Mcu {
            host_pseudonym: host_pseudonym.clone(),
        });
    }

    if host_pseudonym == my_pseudonym {
        let _ = crate::services::voice::session::start_mcu_loop(state);
    } else {
        crate::services::voice::session::stop_mcu_loop(state).await;
    }

    let mut ve = state.voice_engine.lock();
    if let Some(ref mut handle) = *ve {
        if handle.community_id.as_deref() == Some(community_id) && handle.channel_id == channel_id {
            let allowed_to_speak = speakers.contains(&my_pseudonym.to_string());
            handle.engine.set_muted(!allowed_to_speak);
            handle
                .muted_flag
                .store(!allowed_to_speak, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
