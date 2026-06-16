//! Friend list state — friends and pending inbound requests.

use std::time::Instant;

use rekindle_types::display::FriendDisplay;

#[derive(Clone, Debug)]
pub struct PendingFriendRequest {
    pub public_key: String,
    pub display_name: String,
    /// Milliseconds since Unix epoch.
    pub received_at: u64,
}

#[derive(Debug)]
pub struct FriendState {
    pub friends: Vec<FriendDisplay>,
    pub pending_requests: Vec<PendingFriendRequest>,
    pub loaded: bool,
    pub loaded_at: Option<Instant>,
}

impl FriendState {
    pub fn new() -> Self {
        Self {
            friends: Vec::new(),
            pending_requests: Vec::new(),
            loaded: false,
            loaded_at: None,
        }
    }
}

impl Default for FriendState {
    fn default() -> Self {
        Self::new()
    }
}

/// Sort rank for presence status. Lower = higher in the list.
pub fn presence_rank(status: &str) -> u8 {
    match status {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        "offline" => 3,
        _ => 4,
    }
}
