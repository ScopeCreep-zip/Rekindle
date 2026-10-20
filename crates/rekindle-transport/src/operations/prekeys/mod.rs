//! The prekey record on both hosts (plan E2.4, D18): ours, which the
//! "prekey maintenance" task keeps (`own`), a friend's, which a first
//! contact reads and picks from (`pick`), and the rejection of an init
//! naming keys we no longer hold, with its one retry (`reject`). The decisions are
//! `rekindle_crypto::signal::prekey_record`'s and the rows
//! `rekindle_db::prekeys`'; this module moves them over the record port.

mod own;
mod pick;
mod reject;

pub use own::{device_list, maintain, prekey_problem, publish, reconcile, status};
pub use pick::pick_bundle;
pub use reject::FirstContact;
pub(crate) use reject::{acknowledged, began_first_contact, on_rejected, refilled, reject_init};

use crate::operations::friend::FriendError;

/// A storage step's failure as the friend operations report it.
fn stored(what: &str, e: impl std::fmt::Display) -> FriendError {
    FriendError::Failed(format!("{what}: {e}"))
}
