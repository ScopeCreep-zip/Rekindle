//! Phase 14.k — voice signaling adapter.
//!
//! Implements `rekindle_voice::signaling::VoiceSignalingDeps` against
//! the live `AppState` + `tauri::AppHandle` + `DbPool` + the existing
//! `services::community::*` cross-subsystem functions (which Phase 17 /
//! 19 / 20 will eventually own). The crate's signaling handlers
//! (voice_join / voice_leave / stage_update / etc.) consume this trait
//! via `Arc<dyn VoiceSignalingDeps>`.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};

use crate::channels::CommunityEvent;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_voice::signaling::{CommunityVoiceEvent, StageChannelInfo, VoiceSignalingDeps};
use rekindle_voice::transport::VoiceTransport;
use tokio::sync::Mutex as AsyncMutex;

use crate::db::DbPool;
use crate::state::{AppState, ChannelType};
use crate::state_helpers;

pub struct VoiceSignalingAdapter {
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
}

impl VoiceSignalingAdapter {
    /// `_pool` accepted to match the construction shape of the other
    /// Phase-14 adapters (calls, voice, dm) even though signaling
    /// doesn't reach SQLite directly — `persist_hand_raise` goes
    /// through `services::community::persist_hand_raise(&AppState, ...)`.
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, _pool: DbPool) -> Arc<Self> {
        Arc::new(Self { state, app_handle })
    }
}

/// Public free-fn facade — dispatch an inbound community voice
/// gossip `ControlPayload` through the crate handler. Used by
/// `services::veilid::control_moderation` (the gossip dispatcher).
/// Spawns the handler so the caller stays sync.
pub fn handle_voice_signaling(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    let Some(pool) = tauri::Manager::try_state::<DbPool>(app_handle) else {
        tracing::error!("handle_voice_signaling: DbPool state missing");
        return;
    };
    let pool = pool.inner().clone();
    let adapter = VoiceSignalingAdapter::new(state.clone(), app_handle.clone(), pool);
    let deps: Arc<dyn VoiceSignalingDeps> = adapter;
    let cid = community_id.to_string();
    let sender = sender_pseudonym.to_string();
    tauri::async_runtime::spawn(async move {
        rekindle_voice::signaling::handle_voice_signaling(deps, &cid, &sender, payload).await;
    });
}

#[async_trait]
impl VoiceSignalingDeps for VoiceSignalingAdapter {
    fn my_pseudonym(&self, community_id: &str) -> Option<String> {
        state_helpers::my_pseudonym_key(&self.state, community_id)
    }

    fn our_route_blob(&self) -> Vec<u8> {
        state_helpers::our_route_blob(&self.state).unwrap_or_default()
    }

    fn stage_channel_info(&self, community_id: &str, channel_id: &str) -> Option<StageChannelInfo> {
        let communities = self.state.communities.read();
        let community = communities.get(community_id)?;
        let channel = community.channels.iter().find(|ch| ch.id == channel_id)?;
        Some(StageChannelInfo {
            is_stage: matches!(channel.channel_type, ChannelType::Stage),
            speakers: channel.stage_speakers.clone(),
            moderator: channel.stage_moderator.clone(),
        })
    }

    fn update_stage_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator: String,
    ) {
        let mut communities = self.state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                if let Some(t) = topic {
                    channel.topic = t;
                }
                channel.stage_speakers = speakers;
                channel.stage_moderator = Some(moderator);
            }
        }
    }

    fn online_voice_members(&self, community_id: &str) -> Vec<(String, Vec<u8>)> {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return Vec::new();
        };
        let Some(gossip) = community.gossip.as_ref() else {
            return Vec::new();
        };
        gossip
            .online_members
            .iter()
            .map(|(pk, m)| (pk.clone(), m.route_blob.clone()))
            .collect()
    }

    fn decode_channel_id(&self, channel_id: &str) -> Option<[u8; 16]> {
        hex::decode(channel_id).ok()?.try_into().ok()
    }

    fn my_permissions(&self, community_id: &str, channel_id: Option<[u8; 16]>) -> u64 {
        let ch_id = channel_id.map(rekindle_types::id::ChannelId);
        state_helpers::my_permissions(&self.state, community_id, ch_id.as_ref())
    }

    fn sender_has_perm(
        &self,
        community_id: &str,
        sender_pseudonym_hex: &str,
        perm_mask: u64,
    ) -> bool {
        use rekindle_governance::permissions::{compute_permissions, has_all_capabilities};
        use rekindle_types::id::PseudonymKey;

        let communities = self.state.communities.read();
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
        has_all_capabilities(perms, perm_mask)
    }

    fn transport_handle(&self) -> Option<Arc<AsyncMutex<VoiceTransport>>> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| h.transport.clone())
    }

    fn voice_engine_channel_id(&self) -> Option<String> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| h.channel_id.clone())
    }

    fn voice_engine_bound_to(&self, community_id: &str, channel_id: &str) -> bool {
        let ve = self.state.voice_engine.lock();
        ve.as_ref().is_some_and(|h| {
            h.community_id.as_deref() == Some(community_id) && h.channel_id == channel_id
        })
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        crate::state_helpers::set_voice_engine_muted(&self.state, muted);
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        crate::state_helpers::set_voice_engine_deafened(&self.state, deafened);
    }

    async fn rotate_voice_mek_for_membership(
        &self,
        community_id: String,
        channel_id: String,
        member_pseudonym: String,
        joined: bool,
    ) {
        if let Err(error) = crate::services::community::rotate_voice_mek_for_membership(
            &self.app_handle,
            &self.state,
            &community_id,
            &channel_id,
            &member_pseudonym,
            joined,
        )
        .await
        {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %error,
                "voice MEK rotation skipped"
            );
        }
    }

    fn send_to_mesh(&self, community_id: &str, envelope: &CommunityEnvelope) {
        if let Err(e) =
            crate::services::community::send_to_mesh(&self.state, community_id, envelope)
        {
            tracing::debug!(community = %community_id, error = %e, "voice send_to_mesh failed");
        }
    }

    fn advertise_media_capabilities(&self, community_id: &str, channel_id: &str) {
        if let Err(e) = crate::services::voice_adapter::advertise_media_capabilities(
            &self.state,
            community_id,
            channel_id,
        ) {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %e,
                "directed media-caps advertise skipped"
            );
        }
    }

    async fn persist_hand_raise(&self, community_id: String, channel_id: String, raised: bool) {
        if let Err(error) = crate::services::community::persist_hand_raise(
            &self.state,
            &community_id,
            &channel_id,
            raised,
        )
        .await
        {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %error,
                "persist hand raise failed"
            );
        }
    }

    fn next_lamport(&self, community_id: &str) -> u64 {
        state_helpers::increment_lamport(&self.state, community_id)
    }

    fn stage_speakers(&self, community_id: &str, channel_id: &str) -> Vec<String> {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.stage_speakers.clone())
            })
            .unwrap_or_default()
    }

    fn start_mcu_loop(&self) {
        // Build a fresh VoiceAdapter (Arc<dyn VoiceSessionDeps>) and
        // delegate to the crate's start_mcu_loop. Pool acquisition
        // is best-effort — if it's missing, we log and skip rather
        // than panic (matches the legacy services::voice::session
        // facade behavior).
        let Some(pool) = tauri::Manager::try_state::<DbPool>(&self.app_handle) else {
            tracing::warn!("start_mcu_loop: DbPool state missing");
            return;
        };
        let pool = pool.inner().clone();
        let adapter = crate::services::voice_adapter::VoiceAdapter::new(
            self.state.clone(),
            self.app_handle.clone(),
            pool,
        );
        let deps: Arc<dyn rekindle_voice::VoiceSessionDeps> = adapter;
        if let Err(e) = rekindle_voice::session::start_mcu_loop(&deps) {
            tracing::warn!(error = %e, "start_mcu_loop failed");
        }
    }

    async fn stop_mcu_loop(&self) {
        let Some(pool) = tauri::Manager::try_state::<DbPool>(&self.app_handle) else {
            return;
        };
        let pool = pool.inner().clone();
        let adapter = crate::services::voice_adapter::VoiceAdapter::new(
            self.state.clone(),
            self.app_handle.clone(),
            pool,
        );
        let deps: Arc<dyn rekindle_voice::VoiceSessionDeps> = adapter;
        rekindle_voice::session::stop_mcu_loop(&deps).await;
    }

    fn send_to_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &rekindle_protocol::dht::community::envelope::CommunityEnvelope,
    ) {
        if let Err(e) = crate::services::community::send_to_channel_peers(
            &self.state,
            community_id,
            channel_id,
            envelope,
        ) {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %e,
                "voice signaling directed send failed",
            );
        }
    }

    fn my_display_name(&self) -> Option<String> {
        let name = crate::state_helpers::identity_display_name(&self.state);
        (!name.is_empty()).then_some(name)
    }

    fn emit_event(&self, event: CommunityVoiceEvent) {
        match event {
            CommunityVoiceEvent::VoiceJoin {
                community_id,
                channel_id,
                pseudonym_key,
                route_blob,
                display_name,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoiceJoin {
                        community_id,
                        channel_id,
                        pseudonym_key,
                        route_blob,
                        display_name,
                    },
                );
            }
            CommunityVoiceEvent::VoiceRosterChanged {
                community_id,
                channel_id,
                pseudonym_key,
                present,
                display_name: _,
                remote_count,
            } => {
                // Session membership is SIGNALING-driven (the transport
                // roster), never media-driven — a VAD-silent peer sends
                // no packets but is fully present. This feeds both the
                // video-session caps slot and the media-ready roster
                // input; the old path hung both off the first received
                // voice packet, so video egress deadlocked on inbound
                // audio.
                let result = if present {
                    crate::services::community::video_session::on_peer_joined(
                        &self.state,
                        &community_id,
                        &channel_id,
                        &pseudonym_key,
                    )
                } else {
                    crate::services::community::video_session::on_peer_left(
                        &self.state,
                        &community_id,
                        &channel_id,
                        &pseudonym_key,
                    )
                };
                if let Err(e) = result {
                    tracing::warn!(error = %e, present, "video_session roster sync failed");
                }
                crate::services::community::media_ready_runtime::update_media_ready(
                    &self.state,
                    &community_id,
                    &channel_id,
                    |i| i.roster_non_empty = remote_count > 0,
                );
            }
            CommunityVoiceEvent::VoiceJoinHandshake {
                community_id,
                channel_id,
                state,
                peer,
                display_name,
            } => {
                // Media-ready input: the three-way handshake stage.
                let handshake = match state.as_str() {
                    "seen" => Some(rekindle_voice::transport::JoinHandshake::Seen),
                    "connected" => Some(rekindle_voice::transport::JoinHandshake::Connected),
                    _ => None,
                };
                if let Some(hs) = handshake {
                    crate::services::community::media_ready_runtime::update_media_ready(
                        &self.state,
                        &community_id,
                        &channel_id,
                        |i| i.handshake = hs,
                    );
                }
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoiceJoinHandshake {
                        community_id,
                        channel_id,
                        state,
                        peer,
                        display_name,
                    },
                );
            }
            CommunityVoiceEvent::VoicePeerConfirmed {
                community_id,
                channel_id,
                pseudonym_key,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoicePeerConfirmed {
                        community_id,
                        channel_id,
                        pseudonym_key,
                    },
                );
            }
            CommunityVoiceEvent::VoiceLeave {
                community_id,
                channel_id,
                pseudonym_key,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoiceLeave {
                        community_id,
                        channel_id,
                        pseudonym_key,
                    },
                );
            }
            CommunityVoiceEvent::VoiceRoster {
                community_id,
                channel_id,
                participants,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoiceRoster {
                        community_id,
                        channel_id,
                        participants: participants
                            .into_iter()
                            .map(|p| {
                                crate::channels::community_channel::VoiceRosterParticipantEvent {
                                    pseudonym_key: p.pseudonym_key,
                                    display_name: p.display_name,
                                }
                            })
                            .collect(),
                    },
                );
            }
            CommunityVoiceEvent::VoiceModeSwitch {
                community_id,
                channel_id,
                mode,
                host_pseudonym,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::VoiceModeSwitch {
                        community_id,
                        channel_id,
                        mode,
                        host_pseudonym,
                    },
                );
            }
            CommunityVoiceEvent::StageUpdate {
                community_id,
                channel_id,
                topic,
                speakers,
                moderator_pseudonym,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::StageUpdate {
                        community_id,
                        channel_id,
                        topic,
                        speakers,
                        moderator_pseudonym,
                    },
                );
            }
            CommunityVoiceEvent::SpeakRequest {
                community_id,
                channel_id,
                requester_pseudonym,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::SpeakRequest {
                        community_id,
                        channel_id,
                        requester_pseudonym,
                    },
                );
            }
            CommunityVoiceEvent::SpeakResponse {
                community_id,
                channel_id,
                requester_pseudonym,
                granted,
                moderator_pseudonym,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::SpeakResponse {
                        community_id,
                        channel_id,
                        requester_pseudonym,
                        granted,
                        moderator_pseudonym,
                    },
                );
            }
            CommunityVoiceEvent::SoundboardPlay {
                community_id,
                channel_id,
                expression_id,
                actor_pseudonym,
            } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "community-event",
                    CommunityEvent::SoundboardPlay {
                        community_id,
                        channel_id,
                        expression_id,
                        actor_pseudonym,
                    },
                );
            }
            CommunityVoiceEvent::UserMuted {
                target_pseudonym,
                muted,
            } => {
                if let Some(scope) = state_helpers::current_voice_scope(&self.state) {
                    crate::event_dispatch::emit_subscription(
                        &self.app_handle,
                        &SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
                            scope,
                            target_pseudonym,
                            muted,
                        }),
                    );
                }
            }
        }
    }

    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>) {
        state_helpers::register_background_handle(&self.state, handle);
    }
}
