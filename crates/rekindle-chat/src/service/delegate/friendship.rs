//! Friendship delegation — friend request lifecycle + enriched friend list.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn send_friend_request(
        &self, target: &str, message: &str,
    ) -> Result<crate::friendship::request::FriendRequestSent, ChatError> {
        self.friendship.send_friend_request(target, message).await
    }

    pub async fn accept_friend_request(
        &self, peer_key: &str,
    ) -> Result<crate::friendship::accept::FriendAccepted, ChatError> {
        self.friendship.accept_friend_request(peer_key).await
    }

    pub async fn reject_friend_request(&self, peer_key: &str) -> Result<(), ChatError> {
        self.friendship.reject_friend_request(peer_key).await
    }

    pub async fn remove_friend(&self, peer_key: &str) -> Result<(), ChatError> {
        self.friendship.remove_friend(peer_key).await
    }

    pub fn list_pending_requests(
        &self,
    ) -> Vec<rekindle_types::session_types::PendingFriendRequest> {
        self.friendship.list_pending()
    }

    /// Enriched friend list — includes presence status and route availability.
    ///
    /// Queries three data sources:
    /// - session_meta.friend_display_names: public_key → display_name
    /// - session_meta.dm_peers: public_key → DM log keys (profile_dht_key proxy)
    /// - pipeline.state().presence: peer status from subscription events
    /// - io.transport().route_blob: whether the peer has an active route
    pub fn list_friends(&self) -> Vec<FriendSummary> {
        let meta = self.session_meta.read();
        let presence_state = self.pipeline.state().read();

        meta.friend_display_names
            .iter()
            .map(|(public_key, display_name)| {
                // Profile DHT key from DM peer state
                let profile_dht_key = meta.dm_peers.get(public_key)
                    .and_then(|p| if p.outbound_log_key.is_empty() { None } else { Some(public_key.clone()) });

                // Presence status from subscription event state
                let (status, last_seen_ms) = match presence_state.presence.friend(public_key) {
                    Some(info) => {
                        let elapsed_ms = info.last_seen.elapsed().as_millis() as u64;
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        (info.status.clone(), Some(now_ms.saturating_sub(elapsed_ms)))
                    }
                    None => ("offline".to_string(), None),
                };

                // Route availability — peer has a published route blob
                let has_route = self.io.transport().route_blob().is_some();

                FriendSummary {
                    public_key: public_key.clone(),
                    display_name: display_name.clone(),
                    status,
                    last_seen_ms,
                    profile_dht_key,
                    has_route,
                }
            })
            .collect()
    }
}

/// Friend entry for list display. Matches `FriendDisplay` in rekindle-types/display.rs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FriendSummary {
    pub public_key: String,
    pub display_name: String,
    pub status: String,
    pub last_seen_ms: Option<u64>,
    pub profile_dht_key: Option<String>,
    pub has_route: bool,
}
