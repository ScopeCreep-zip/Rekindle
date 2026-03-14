//! Voice signaling handlers for community gossip payloads.
//!
//! Extracted from `veilid_service.rs` to keep that file under the line limit
//! and group all voice-related gossip handling in one place.

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
        ControlPayload::VoiceJoin { channel_id, route_blob } => {
            handle_voice_join(app_handle, state, community_id, sender_pseudonym, channel_id, route_blob);
        }
        ControlPayload::VoiceLeave { channel_id } => {
            handle_voice_leave(app_handle, state, community_id, sender_pseudonym, channel_id);
        }
        ControlPayload::VoiceModeSwitch { channel_id, mode, host_pseudonym } => {
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
        ControlPayload::VoiceMute { channel_id: _, target_pseudonym, muted } => {
            handle_voice_mute(app_handle, state, community_id, &target_pseudonym, muted);
        }
        ControlPayload::VoiceDeafen { channel_id: _, target_pseudonym, deafened } => {
            handle_voice_deafen(state, community_id, &target_pseudonym, deafened);
        }
        ControlPayload::VoiceRoster { channel_id: _, participants } => {
            handle_voice_roster(state, participants);
        }
        _ => {}
    }
}

fn handle_voice_join(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    channel_id: String,
    route_blob: Vec<u8>,
) {
    let (shared_transport, my_pseudonym) = {
        let ve = state.voice_engine.lock();
        let transport = ve.as_ref().map(|h| h.transport.clone());
        drop(ve);
        let pk = {
            let communities = state.communities.read();
            communities.get(community_id)
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
            let (peer_count, current_mode) = {
                let mut t = transport.lock().await;
                if let Err(e) = t.add_peer(&sender_key, &blob) {
                    tracing::warn!(peer = %sender_key, error = %e, "failed to add voice peer");
                }
                (t.peer_count(), t.mode().clone())
            };

            // Auto-elect MCU host at 6+ participants (5 peers + self)
            if peer_count >= 5 && matches!(current_mode, rekindle_voice::VoiceMode::Mesh) {
                let mut candidates = {
                    let t = transport.lock().await;
                    t.peer_keys()
                };
                candidates.push(my_pk.clone());
                candidates.sort();
                let elected_host = candidates[0].clone();

                tracing::info!(
                    peer_count = peer_count + 1,
                    host = %elected_host,
                    "auto-electing MCU host (6+ participants)"
                );

                let mode_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                    ControlPayload::VoiceModeSwitch {
                        channel_id: ch_id.clone(),
                        mode: "mcu".to_string(),
                        host_pseudonym: Some(elected_host.clone()),
                    },
                );
                let _ = crate::commands::community::send_to_mesh(&state, &cid, &mode_envelope);

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
                t.peer_keys().iter().filter_map(|pk| {
                    let gossip = cs?.gossip.as_ref()?;
                    let member = gossip.online_members.get(pk)?;
                    Some(rekindle_protocol::dht::community::envelope::VoiceRosterEntry {
                        pseudonym_key: pk.clone(),
                        route_blob: member.route_blob.clone(),
                        muted: false,
                        deafened: false,
                    })
                }).collect()
            };
            if !participants.is_empty() {
                let roster_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                    ControlPayload::VoiceRoster {
                        channel_id: ch_id,
                        participants,
                    },
                );
                let _ = crate::commands::community::send_to_mesh(&state, &cid, &roster_envelope);
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
    let (shared_transport, my_pseudonym) = {
        let ve = state.voice_engine.lock();
        let transport = ve.as_ref().map(|h| h.transport.clone());
        drop(ve);
        let pk = {
            let communities = state.communities.read();
            communities.get(community_id)
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
            let (peer_count, current_mode) = {
                let mut t = transport.lock().await;
                t.remove_peer(&sender_key);
                (t.peer_count(), t.mode().clone())
            };

            if let rekindle_voice::VoiceMode::Mcu { ref host_pseudonym } = current_mode {
                if *host_pseudonym == sender_key {
                    // MCU host left — fall back to mesh
                    tracing::info!("MCU host left — falling back to mesh");
                    {
                        let mut t = transport.lock().await;
                        t.set_mode(rekindle_voice::VoiceMode::Mesh);
                    }
                    crate::services::voice::session::stop_mcu_loop(&state).await;

                    let mesh_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                        ControlPayload::VoiceModeSwitch {
                            channel_id: ch_id.clone(),
                            mode: "mesh".to_string(),
                            host_pseudonym: None,
                        },
                    );
                    let _ = crate::commands::community::send_to_mesh(&state, &cid, &mesh_envelope);

                    // Re-elect if still 6+ participants
                    if peer_count >= 5 {
                        let mut candidates = {
                            let t = transport.lock().await;
                            t.peer_keys()
                        };
                        candidates.push(my_pk.clone());
                        candidates.sort();
                        let elected_host = candidates[0].clone();

                        let mode_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                            ControlPayload::VoiceModeSwitch {
                                channel_id: ch_id,
                                mode: "mcu".to_string(),
                                host_pseudonym: Some(elected_host.clone()),
                            },
                        );
                        let _ = crate::commands::community::send_to_mesh(&state, &cid, &mode_envelope);

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
                } else if peer_count < 5 {
                    // Below threshold — switch back to mesh
                    tracing::info!(peer_count, "below MCU threshold — switching to mesh");
                    {
                        let mut t = transport.lock().await;
                        t.set_mode(rekindle_voice::VoiceMode::Mesh);
                    }
                    crate::services::voice::session::stop_mcu_loop(&state).await;

                    let mesh_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                        ControlPayload::VoiceModeSwitch {
                            channel_id: ch_id,
                            mode: "mesh".to_string(),
                            host_pseudonym: None,
                        },
                    );
                    let _ = crate::commands::community::send_to_mesh(&state, &cid, &mesh_envelope);
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
        communities.get(community_id).and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_muted(muted);
            handle.muted_flag.store(muted, std::sync::atomic::Ordering::Relaxed);
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
        communities.get(community_id).and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(target_pseudonym) {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_deafened(deafened);
            handle.deafened_flag.store(deafened, std::sync::atomic::Ordering::Relaxed);
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
