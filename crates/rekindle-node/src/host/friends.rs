//! The friend projection and 1:1 receive path as the GUI reaches them in
//! process (plan E2.1), until each unported reader's own step moves it into
//! the Host.

use std::sync::Arc;

use rekindle_transport::operations::direct::{self, Routed};
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::friend::FriendState;

use super::adapters::friends::HostFriends;
use super::runtime::Host;

impl Host {
    /// Every friend, any friendship state (read-only).
    #[must_use]
    pub fn friends(&self) -> Vec<FriendState> {
        self.ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.friends.values().cloned().collect())
            .unwrap_or_default()
    }

    /// One friend (read-only).
    #[must_use]
    pub fn friend(&self, public_key: &str) -> Option<FriendState> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.friends.get(public_key).cloned())
    }

    /// The friend operations' Host, for the GUI's unported callers of the
    /// 1:1 send path (`operations::dm`).
    #[must_use]
    pub fn friend_host(&self) -> Arc<dyn FriendHost> {
        Arc::new(HostFriends::of(&self.ctx))
    }

    /// The one friend presence adapter, for the GUI's friend sync (until
    /// E2.5) and its watch dispatch.
    #[must_use]
    pub fn friend_presence(&self) -> Arc<HostFriends> {
        Arc::new(HostFriends::of(&self.ctx))
    }

    /// The network reattached: every friend with a profile record is marked
    /// unwatched, so the friend sync re-establishes the watches; how many.
    pub fn invalidate_friend_watches(&self) -> usize {
        let peers: Vec<String> = self
            .friends()
            .into_iter()
            .filter(|f| f.dht_record_key.is_some())
            .map(|f| f.public_key)
            .collect();
        let count = peers.len();
        self.ctx.friend_watches.mark_unwatched(peers);
        count
    }

    /// Handle a `'D'` app message the GUI's Veilid loop received.
    pub async fn receive_direct(&self, datagram: &[u8]) -> Routed {
        direct::receive(&HostFriends::of(&self.ctx), datagram).await
    }

    /// Handle a `'D'` app call the GUI's Veilid loop received.
    pub async fn receive_direct_call(&self, datagram: &[u8]) -> direct::Called {
        direct::answer_call(&HostFriends::of(&self.ctx), datagram).await
    }
}
