//! Community dispatch handlers, split by concern.
//!
//! All handlers are `pub(crate)` and re-exported here so
//! `dispatch::community::handle_*` call sites in the router stay
//! byte-identical.

mod lifecycle;
mod membership;
mod query;
mod successor;
mod transfer;

pub(crate) use lifecycle::{
    activate_community, handle_create, handle_creator_round, handle_forget, handle_join,
    handle_leave, handle_update_info, CreatorRound,
};
pub(crate) use membership::{handle_approve, handle_pending_members, handle_reject};
pub(crate) use query::{
    handle_co_creation, handle_info, handle_list, handle_members, handle_views,
};
pub(crate) use successor::{handle_follow_successor, handle_publish_successor, handle_successors};
pub(crate) use transfer::{
    handle_accept_transfer, handle_offer_transfer, handle_withdraw_transfer,
};
