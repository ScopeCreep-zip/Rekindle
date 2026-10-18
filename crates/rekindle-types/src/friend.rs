//! Our own identity as the frontends show it, and the one status vocabulary
//! a friend and we are described in (plan E1.1). `UserStatus` is
//! [`SessionStatus`](crate::presence::SessionStatus): friends and community
//! presence speak one language, Invisible folding to Offline on the wire.

use serde::{Deserialize, Serialize};

pub use crate::presence::SessionStatus as UserStatus;

/// The unlocked identity: who we are and the status we show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityState {
    pub public_key: String,
    pub display_name: String,
    pub status: UserStatus,
    pub status_message: String,
}
