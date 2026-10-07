//! Voice signaling events → the frontend's voice subscription events,
//! and the desktop state those events drive (video session caps, media
//! ready).

use std::sync::Arc;

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceScope};
use rekindle_voice::signaling::CommunityVoiceEvent;

use crate::state::AppState;
use crate::state_helpers;

/// Every voice-signalling emit names the same kind of scope.
fn community_scope(community: String, channel: String) -> VoiceScope {
    VoiceScope::Community { community, channel }
}

/// Deliver one signaling event.
pub(super) fn emit(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    event: CommunityVoiceEvent,
) {
    match event {
        CommunityVoiceEvent::VoiceJoin {
            community_id,
            channel_id,
            pseudonym_key,
            route_blob,
            display_name,
        } => {
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::Joined {
                    scope: community_scope(community_id, channel_id),
                    pseudonym: pseudonym_key,
                    display_name,
                    route_blob: Some(route_blob),
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
                    state,
                    &community_id,
                    &channel_id,
                    &pseudonym_key,
                )
            } else {
                crate::services::community::video_session::on_peer_left(
                    state,
                    &community_id,
                    &channel_id,
                    &pseudonym_key,
                )
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, present, "video_session roster sync failed");
            }
            crate::services::community::media_ready_runtime::update_media_ready(
                state,
                &community_id,
                &channel_id,
                |i| i.roster_non_empty = remote_count > 0,
            );
        }
        CommunityVoiceEvent::VoiceJoinHandshake {
            community_id,
            channel_id,
            state: stage,
            peer,
            display_name,
        } => {
            // Media-ready input: the three-way handshake stage.
            let handshake = match stage.as_str() {
                "seen" => Some(rekindle_voice::transport::JoinHandshake::Seen),
                "connected" => Some(rekindle_voice::transport::JoinHandshake::Connected),
                _ => None,
            };
            if let Some(hs) = handshake {
                crate::services::community::media_ready_runtime::update_media_ready(
                    state,
                    &community_id,
                    &channel_id,
                    |i| i.handshake = hs,
                );
            }
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::JoinHandshake {
                    scope: community_scope(community_id, channel_id),
                    state: stage,
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
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::PeerConfirmed {
                    scope: community_scope(community_id, channel_id),
                    pseudonym: pseudonym_key,
                },
            );
        }
        CommunityVoiceEvent::VoiceLeave {
            community_id,
            channel_id,
            pseudonym_key,
        } => {
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::Left {
                    scope: community_scope(community_id, channel_id),
                    pseudonym: pseudonym_key,
                },
            );
        }
        CommunityVoiceEvent::VoiceRoster {
            community_id,
            channel_id,
            participants,
        } => {
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::RosterUpdated {
                    scope: community_scope(community_id, channel_id),
                    participants: participants
                        .into_iter()
                        .map(|p| rekindle_types::subscription_events::VoiceParticipant {
                            pseudonym_key: p.pseudonym_key,
                            display_name: p.display_name,
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
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::ModeChanged {
                    scope: community_scope(community_id, channel_id),
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
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::StageUpdated {
                    scope: community_scope(community_id, channel_id),
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
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::SpeakRequested {
                    scope: community_scope(community_id, channel_id),
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
            crate::event_dispatch::emit_voice(
                app_handle,
                rekindle_types::subscription_events::VoiceEvent::SpeakResponded {
                    scope: community_scope(community_id, channel_id),
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
            crate::event_dispatch::emit_voice(
                app_handle,
                VoiceEvent::SoundboardPlayed {
                    scope: community_scope(community_id, channel_id),
                    expression_id,
                    actor_pseudonym,
                },
            );
        }
        CommunityVoiceEvent::UserMuted {
            target_pseudonym,
            muted,
        } => {
            if let Some(scope) = state_helpers::current_voice_scope(state) {
                crate::event_dispatch::emit_subscription(
                    app_handle,
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
