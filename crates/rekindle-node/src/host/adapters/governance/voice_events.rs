//! Project daemon voice gossip with names from the Host's verified card view.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_types::key_format::pseudonym_hex;
use rekindle_types::subscription_events::{
    SubscriptionEvent, VoiceEvent, VoiceParticipant, VoiceScope,
};

use super::HostGovernance;

impl HostGovernance {
    pub(crate) fn named_voice_event(
        &self,
        community: &str,
        sender: &str,
        payload: &ControlPayload,
    ) -> Option<SubscriptionEvent> {
        let scope = |channel: &str| VoiceScope::Community {
            community: community.to_owned(),
            channel: channel.to_owned(),
        };
        let event = match payload {
            ControlPayload::VoiceJoin {
                channel_id,
                route_blob,
                ..
            } => {
                pseudonym_hex(sender).ok()?;
                VoiceEvent::Joined {
                    scope: scope(channel_id),
                    pseudonym: sender.to_owned(),
                    display_name: self.member_shown_name(community, sender),
                    route_blob: Some(route_blob.clone()),
                }
            }
            ControlPayload::VoiceRoster {
                channel_id,
                participants,
            } => VoiceEvent::RosterUpdated {
                scope: scope(channel_id),
                participants: participants
                    .iter()
                    .filter_map(|peer| {
                        pseudonym_hex(&peer.pseudonym_key).ok()?;
                        Some(VoiceParticipant {
                            display_name: self.member_shown_name(community, &peer.pseudonym_key),
                            pseudonym_key: peer.pseudonym_key.clone(),
                        })
                    })
                    .collect(),
            },
            _ => return None,
        };
        Some(SubscriptionEvent::Voice(event))
    }
}

#[cfg(test)]
#[path = "voice_events_tests.rs"]
mod tests;
