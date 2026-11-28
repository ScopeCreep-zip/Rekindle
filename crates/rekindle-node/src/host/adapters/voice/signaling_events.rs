//! Project signaling and the authoritative roster onto the Host journal.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceScope};
use rekindle_voice::signaling::CommunityVoiceEvent;

use super::deps::HostVoiceDeps;

/// Every voice-signalling emit names the same kind of scope.
fn community_scope(community: String, channel: String) -> VoiceScope {
    VoiceScope::Community { community, channel }
}

/// The session-state key of the call's remote roster in the host's journal.
pub(crate) const ROSTER_STATE: &str = "voice.roster";

/// Publish the roster signaling now holds as the call's current state
/// (plan C7.29; ADR 0013): every window gets it live, and a window that
/// subscribes later is handed it before the live changes, so no window's
/// list depends on having seen each add as it happened.
fn publish_roster(
    deps: &HostVoiceDeps,
    community: String,
    channel: String,
    roster: Vec<rekindle_voice::signaling::VoiceRosterParticipant>,
) {
    deps.ctx()
        .journal
        .publish_state(ROSTER_STATE, roster_event(community, channel, roster));
}

/// The roster as the shared vocabulary's `RosterUpdated`.
fn roster_event(
    community: String,
    channel: String,
    roster: Vec<rekindle_voice::signaling::VoiceRosterParticipant>,
) -> SubscriptionEvent {
    SubscriptionEvent::Voice(VoiceEvent::RosterUpdated {
        scope: community_scope(community, channel),
        participants: roster,
    })
}

/// Deliver one signaling event.
pub(super) fn emit(deps: &HostVoiceDeps, event: CommunityVoiceEvent) {
    match event {
        CommunityVoiceEvent::VoiceJoin {
            community_id,
            channel_id,
            pseudonym_key,
            route_blob,
            display_name,
        } => {
            deps.publish(VoiceEvent::Joined {
                scope: community_scope(community_id, channel_id),
                pseudonym: pseudonym_key,
                display_name,
                route_blob: Some(route_blob),
            });
        }
        CommunityVoiceEvent::VoiceRosterChanged {
            community_id,
            channel_id,
            pseudonym_key,
            present,
            display_name: _,
            roster,
        } => {
            let _ = (pseudonym_key, present);
            publish_roster(deps, community_id, channel_id, roster);
        }
        CommunityVoiceEvent::VoiceJoinHandshake {
            community_id,
            channel_id,
            state: stage,
            media_ready,
            peer,
            display_name,
        } => {
            let scope = community_scope(community_id, channel_id);
            if let Some(media_ready) = media_ready {
                deps.publish(VoiceEvent::MediaReady {
                    scope: scope.clone(),
                    ready: media_ready.ready,
                    reason: media_ready.reason,
                });
            }
            deps.publish(VoiceEvent::JoinHandshake {
                scope,
                state: stage,
                peer,
                display_name,
            });
        }
        CommunityVoiceEvent::VoicePeerConfirmed {
            community_id,
            channel_id,
            pseudonym_key,
        } => {
            deps.publish(VoiceEvent::PeerConfirmed {
                scope: community_scope(community_id, channel_id),
                pseudonym: pseudonym_key,
            });
        }
        CommunityVoiceEvent::VoiceLeave {
            community_id,
            channel_id,
            pseudonym_key,
        } => {
            deps.publish(VoiceEvent::Left {
                scope: community_scope(community_id, channel_id),
                pseudonym: pseudonym_key,
            });
        }
        CommunityVoiceEvent::VoiceRoster {
            community_id,
            channel_id,
            participants,
        } => {
            deps.ctx()
                .publish_event(roster_event(community_id, channel_id, participants));
        }
        CommunityVoiceEvent::VoiceModeSwitch {
            community_id,
            channel_id,
            mode,
            host_pseudonym,
        } => {
            deps.publish(VoiceEvent::ModeChanged {
                scope: community_scope(community_id, channel_id),
                mode,
                host_pseudonym,
            });
        }
        CommunityVoiceEvent::StageUpdate {
            community_id,
            channel_id,
            topic,
            speakers,
            moderator_pseudonym,
        } => {
            deps.publish(VoiceEvent::StageUpdated {
                scope: community_scope(community_id, channel_id),
                topic,
                speakers,
                moderator_pseudonym,
            });
        }
        CommunityVoiceEvent::SpeakRequest {
            community_id,
            channel_id,
            requester_pseudonym,
        } => {
            deps.publish(VoiceEvent::SpeakRequested {
                scope: community_scope(community_id, channel_id),
                requester_pseudonym,
            });
        }
        CommunityVoiceEvent::SpeakResponse {
            community_id,
            channel_id,
            requester_pseudonym,
            granted,
            moderator_pseudonym,
        } => {
            deps.publish(VoiceEvent::SpeakResponded {
                scope: community_scope(community_id, channel_id),
                requester_pseudonym,
                granted,
                moderator_pseudonym,
            });
        }
        CommunityVoiceEvent::SoundboardPlay {
            community_id,
            channel_id,
            expression_id,
            actor_pseudonym,
        } => {
            deps.publish(VoiceEvent::SoundboardPlayed {
                scope: community_scope(community_id, channel_id),
                expression_id,
                actor_pseudonym,
            });
        }
        CommunityVoiceEvent::UserMuted {
            target_pseudonym,
            muted,
        } => {
            if let Some(scope) = deps.voice().session() {
                deps.publish(VoiceEvent::MuteChanged {
                    scope,
                    target_pseudonym,
                    muted,
                });
            }
        }
    }
}
