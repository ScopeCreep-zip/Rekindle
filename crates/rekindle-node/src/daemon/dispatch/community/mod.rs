//! Community dispatch handlers, split by concern.
//!
//! All handlers are `pub(crate)` and re-exported here so
//! `dispatch::community::handle_*` call sites in the router stay
//! byte-identical.

mod lifecycle;
mod membership;
mod ownership;
mod query;

pub(crate) use lifecycle::{handle_create, handle_join, handle_leave};
pub(crate) use membership::{handle_approve, handle_pending_members, handle_reject};
pub(crate) use ownership::handle_transfer_ownership;
pub(crate) use query::{handle_info, handle_list};
