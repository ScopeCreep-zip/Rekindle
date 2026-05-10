//! Friendship delegation — friend request lifecycle.

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

    pub fn list_friends(&self) -> Vec<(String, String)> {
        self.session_meta.read()
            .friend_display_names
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}
