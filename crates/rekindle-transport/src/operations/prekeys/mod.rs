//! The prekey record on both hosts (plan E2.4, D18): ours, which the
//! "prekey maintenance" task keeps (`own`), a friend's, which a first
//! contact reads and picks from (`pick`), and the rejection of an init
//! naming keys we no longer hold, with its one retry (`reject`), and the
//! first contact every session opened from a friend's record goes through,
//! with its visible failure and Retry (`first_contact`). The decisions are
//! `rekindle_crypto::signal::prekey_record`'s and the rows
//! `rekindle_db::prekeys`'; this module moves them over the record port.

mod first_contact;
mod own;
mod pick;
mod reject;

pub use first_contact::retry as retry_first_contact;
pub(crate) use first_contact::{
    bundle_for_friend, failed, open_first_contact, open_or_fail, refused,
};
pub use own::{device_list, maintain, plan_and_store, publish, publish_landed, reconcile, status};
pub use pick::pick_bundle;
pub use reject::FirstContact;
pub(crate) use reject::{acknowledged, began_first_contact, on_rejected, refilled, reject_init};
