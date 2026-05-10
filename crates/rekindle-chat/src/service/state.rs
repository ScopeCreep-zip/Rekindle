//! ChatService state query methods — unread, typing, presence, voice, identity.

use rekindle_types::session_types::SessionIdentity;
use rekindle_types::subscription_events::{SubscriptionEvent, UnreadContext};

use crate::events::state::{PresenceInfo, VoiceParticipantInfo};

use super::ChatService;

impl ChatService {
    pub fn unread_channels(&self) -> std::collections::HashMap<(String, String), u32> {
        self.pipeline.state().read().unread.channels.clone()
    }

    pub fn unread_dms(&self) -> std::collections::HashMap<String, u32> {
        self.pipeline.state().read().unread.dms.clone()
    }

    pub fn unread_friend_requests(&self) -> u32 {
        self.pipeline.state().read().unread.friend_requests
    }

    pub fn mark_channel_read(&self, community: &str, channel: &str) {
        let prev = self.pipeline.state().write().unread.mark_channel_read(community, channel);
        if prev > 0 {
            self.pipeline.process(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::Channel {
                    community: community.into(),
                    channel: channel.into(),
                },
                count: 0,
            });
        }
    }

    pub fn mark_dm_read(&self, peer_key: &str) {
        let prev = self.pipeline.state().write().unread.mark_dm_read(peer_key);
        if prev > 0 {
            self.pipeline.process(SubscriptionEvent::UnreadChanged {
                context: UnreadContext::Dm { peer_key: peer_key.into() },
                count: 0,
            });
        }
    }

    pub fn typing_in_channel(&self, community: &str, channel: &str) -> Vec<String> {
        self.pipeline.state().write().typing.channel_typers(community, channel)
    }

    pub fn typing_in_dm(&self, peer_key: &str) -> bool {
        self.pipeline.state().read().typing.is_dm_typing(peer_key)
    }

    pub fn presence(&self, community: &str) -> Vec<(String, PresenceInfo)> {
        self.pipeline.state().read().presence.community_members(community)
    }

    pub fn friend_presence(&self, peer_key: &str) -> Option<PresenceInfo> {
        self.pipeline.state().read().presence.friend(peer_key).cloned()
    }

    pub fn voice_participants(&self, community: &str, channel: &str) -> Vec<VoiceParticipantInfo> {
        self.pipeline.state().write().voice.participants(community, channel)
    }

    pub fn session_identity(&self) -> Option<SessionIdentity> {
        self.session_meta.read().identity.clone()
    }

    pub fn watch_count(&self) -> usize {
        self.watches.count()
    }

    pub fn community_count(&self) -> usize {
        self.session_meta.read().communities.len()
    }

    pub fn friend_count(&self) -> usize {
        self.session_meta.read().friend_display_names.len()
    }

}
