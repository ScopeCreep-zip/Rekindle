//! Friend DTOs (plan E2.1): what `FriendList`, `FriendRequests`,
//! `FriendBlockedList`, `FriendInviteList`, `FriendInviteCreate` and
//! `IdentitySafetyNumber` (plan E2.3) answer.

use serde::{Deserialize, Serialize};

use crate::friend::{FriendshipState, GameInfoState, UserStatus};

/// The group a friend in no group of ours is listed under.
pub const DEFAULT_FRIEND_GROUP: &str = "Friends";

/// The group the buddy list shows a friend in `group` under: the group, or
/// [`DEFAULT_FRIEND_GROUP`] for none.
#[must_use]
pub fn group_label(group: Option<&str>) -> String {
    group.unwrap_or(DEFAULT_FRIEND_GROUP).to_string()
}

/// A friend on the buddy list. Presence is shown only for an accepted
/// friend; a pending one reads offline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: UserStatus,
    pub status_message: Option<String>,
    pub game_info: Option<GameInfoState>,
    pub group: Option<String>,
    /// The group the buddy list shows them under: `group`, or
    /// [`DEFAULT_FRIEND_GROUP`] for none.
    pub group_label: String,
    pub unread_count: u32,
    pub last_seen_at: Option<i64>,
    pub friendship_state: FriendshipState,
    /// Why the secure session could not be set up, while it is failed.
    pub handshake_error: Option<String>,
    /// Our first contact is being resent from a fresh key (plan E2.4).
    pub handshake_retrying: bool,
    /// Their keys are not signed by the identity we know for them: a trust
    /// alert, shown on their row until it is resolved (plan E2.4, B3).
    pub identity_pin_mismatch: bool,
}

impl Friend {
    /// The buddy list's view of `friend`.
    #[must_use]
    pub fn of(friend: &crate::friend::FriendState) -> Self {
        let accepted = friend.friendship_state == FriendshipState::Accepted;
        Self {
            public_key: friend.public_key.clone(),
            display_name: friend.display_name.clone(),
            nickname: friend.nickname.clone(),
            status: if accepted {
                friend.status
            } else {
                UserStatus::Offline
            },
            status_message: friend.status_message.clone().filter(|_| accepted),
            game_info: friend.game_info.clone().filter(|_| accepted),
            group: friend.group.clone(),
            group_label: group_label(friend.group.as_deref()),
            unread_count: friend.unread_count,
            last_seen_at: friend.last_seen_at,
            friendship_state: friend.friendship_state,
            handshake_error: friend.handshake_error.clone(),
            handshake_retrying: friend.handshake_retrying,
            identity_pin_mismatch: friend.identity_pin_mismatch,
        }
    }
}

/// An incoming friend request, waiting for our answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub public_key: String,
    pub display_name: String,
    pub message: String,
    pub received_at: i64,
}

/// A blocked identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedUser {
    pub public_key: String,
    pub display_name: String,
    pub blocked_at: i64,
}

/// A friend invite we made that is still open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingInvite {
    pub invite_id: String,
    pub url: String,
    pub created_at: i64,
    pub expires_at: i64,
    /// `pending`, or `responded` once someone's request answered it.
    pub status: String,
    pub accepted_by: Option<String>,
}

/// A new friend invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreated {
    pub url: String,
    pub invite_id: String,
}

/// Our safety number with a peer and whether we verified it (plan E2.3):
/// what `IdentitySafetyNumber` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyNumberView {
    pub peer_key: String,
    /// The 60 digits both sides see, shown as 12 groups of 5.
    pub digits: String,
    /// Signal's scannable form (`CombinedFingerprints`), base64.
    #[serde(with = "crate::base64_bytes")]
    pub scannable: Vec<u8>,
    /// The scannable form as a QR code: one string per row, `1` a dark
    /// module, without the quiet zone.
    pub qr_rows: Vec<String>,
    pub verified: bool,
    pub verified_at: Option<i64>,
    /// The key we verified changed and the change is not acknowledged
    /// yet: the next action toward them is reviewed (`VerifiedKeyChanged`).
    pub changed: bool,
}
