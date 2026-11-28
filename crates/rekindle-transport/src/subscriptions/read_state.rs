//! Read-only state queries: friend requests, typing, presence, voice
//! rosters, and watch/mesh introspection.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use super::{state, SubscriptionManager};
use crate::gossip::GossipMesh;

impl SubscriptionManager {
    /// Pending inbound friend requests. Channel and DM counts are the
    /// Host's, derived from the read markers (plan E3.1, D25).
    pub fn unread_friend_requests(&self) -> u32 {
        self.state.read().friend_requests
    }

    /// Active typers in a channel.
    pub fn typing_in_channel(&self, community: &str, channel: &str) -> Vec<String> {
        self.state.write().typing.channel_typers(community, channel)
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

    /// Active dedup digest entries (for diagnostics).
    pub fn dedup_entries(&self) -> usize {
        self.dedup.read().len()
    }

    /// Total cross-tier duplicates suppressed since this manager started
    /// (for diagnostics).
    pub fn dedup_suppressed(&self) -> u64 {
        self.dedup.read().suppressed_count()
    }

    /// Access the shared gossip meshes (for BroadcastManager).
    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> {
        &self.meshes
    }
}
