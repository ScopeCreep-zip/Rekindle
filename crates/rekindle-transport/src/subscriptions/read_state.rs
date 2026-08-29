//! Read-only state queries: unread counts, typing, presence, voice
//! rosters, and watch/mesh introspection.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use super::events::{self, SubscriptionEvent};
use super::{state, SubscriptionManager};
use crate::gossip::GossipMesh;

impl SubscriptionManager {
    /// Read-only access to unread state.
    pub fn unread_channels(&self) -> HashMap<(String, String), u32> {
        self.state.read().unread.channels.clone()
    }

    pub fn unread_dms(&self) -> HashMap<String, u32> {
        self.state.read().unread.dms.clone()
    }

    pub fn unread_friend_requests(&self) -> u32 {
        self.state.read().unread.friend_requests
    }

    /// Mark a channel as read.
    pub fn mark_channel_read(&self, community: &str, channel: &str) {
        let prev = self
            .state
            .write()
            .unread
            .mark_channel_read(community, channel);
        if prev > 0 {
            self.process_event(SubscriptionEvent::UnreadChanged {
                context: events::UnreadContext::Channel {
                    community: community.into(),
                    channel: channel.into(),
                },
                count: 0,
            });
        }
    }

    /// Mark a DM conversation as read.
    pub fn mark_dm_read(&self, peer_key: &str) {
        let prev = self.state.write().unread.mark_dm_read(peer_key);
        if prev > 0 {
            self.process_event(SubscriptionEvent::UnreadChanged {
                context: events::UnreadContext::Dm {
                    peer_key: peer_key.into(),
                },
                count: 0,
            });
        }
    }

    /// Active typers in a channel.
    pub fn typing_in_channel(&self, community: &str, channel: &str) -> Vec<String> {
        self.state.write().typing.channel_typers(community, channel)
    }

    /// Whether a peer is typing in DM.
    pub fn typing_in_dm(&self, peer_key: &str) -> bool {
        self.state.read().typing.is_dm_typing(peer_key)
    }

    /// Community member presence.
    pub fn presence(&self, community: &str) -> Vec<(String, state::PresenceInfo)> {
        self.state.read().presence.community_members(community)
    }

    /// Friend presence.
    pub fn friend_presence(&self, peer_key: &str) -> Option<state::PresenceInfo> {
        self.state.read().presence.friend(peer_key).cloned()
    }

    /// Voice channel participants.
    pub fn voice_participants(
        &self,
        community: &str,
        channel: &str,
    ) -> Vec<state::VoiceParticipantInfo> {
        self.state.write().voice.participants(community, channel)
    }

    /// Total active watch count (for diagnostics).
    pub fn watch_count(&self) -> usize {
        self.watches.read().count()
    }

    /// Access the shared gossip meshes (for BroadcastManager).
    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> {
        &self.meshes
    }
}
