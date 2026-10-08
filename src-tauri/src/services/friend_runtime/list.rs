//! Phase 23.C — get_friends orchestrator lifted from
//! `commands/friends.rs`. Pure read of `state.friends` projected into
//! the `FriendResponse` DTO; presence fields gated on Accepted state.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::state::{AppState, FriendshipState, GameInfoState, UserStatus};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendResponse {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: UserStatus,
    pub status_message: Option<String>,
    pub game_info: Option<GameInfoState>,
    pub group: Option<String>,
    pub unread_count: u32,
    pub last_seen_at: Option<i64>,
    pub friendship_state: FriendshipState,
}

pub fn list_friends_inner(state: &Arc<AppState>) -> Vec<FriendResponse> {
    let friends = state.friends.read();
    friends
        .values()
        .filter(|f| !matches!(f.friendship_state, FriendshipState::Removing))
        .map(|f| {
            let is_accepted = f.friendship_state == FriendshipState::Accepted;
            FriendResponse {
                public_key: f.public_key.clone(),
                display_name: f.display_name.clone(),
                nickname: f.nickname.clone(),
                status: if is_accepted {
                    f.status
                } else {
                    UserStatus::Offline
                },
                status_message: if is_accepted {
                    f.status_message.clone()
                } else {
                    None
                },
                game_info: if is_accepted {
                    f.game_info.clone()
                } else {
                    None
                },
                group: f.group.clone(),
                unread_count: f.unread_count,
                last_seen_at: f.last_seen_at,
                friendship_state: f.friendship_state,
            }
        })
        .collect()
}
