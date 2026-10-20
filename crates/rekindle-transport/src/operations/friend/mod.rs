//! Friends on both hosts (plan E2.1): adding by key or invite, answering
//! requests, removing and blocking, our labels for them, invites, session
//! resets, safety numbers and verification, and the receive side of the
//! handshake. Everything the Host owns
//! comes through [`FriendHost`].

mod add;
mod answer;
mod blocked;
mod edit;
mod error;
mod host;
pub mod identity_rotation;
pub(crate) mod inbound;
mod invites;
mod list;
mod list_record;
mod remove;
mod rotate;
pub mod session_reset;
pub use session_reset::expire_resets;
pub(crate) mod state;
pub mod verification;

#[cfg(test)]
mod handshake_size_tests;

pub use add::{add, from_invite};
pub use answer::{accept, cancel, expire_stale_requests, reject};
pub use blocked::{block, is_blocked, list as blocked, unblock};
pub use edit::{create_group, delete_group, move_to_group, rename_group, set_nickname};
pub use error::FriendError;
pub use host::{BackendHold, FriendHost, FriendStores};
pub use invites::{
    cancel as cancel_invite, create as create_invite, expire_stale as expire_stale_invites,
    list as invites, open as open_invite, MAX_INVITE_AGE_SECS,
};
pub use list::{friends, pending};
pub use list_record::push as push_friend_list;
pub use remove::{forget_removed, remove};
pub use rotate::{notify_profile_key_rotated, rotate_profile_key};
