//! The Host's community model and its one store (plan E3.1).

pub mod sealed_pages;
pub mod state;
pub mod store;

#[cfg(test)]
mod store_tests;

pub use state::{
    display_role_name, CategoryInfo, ChannelInfo, ChannelType, CommunityRecords, CommunityState,
    GossipOverlay, MemberProfileSnapshot, MemberRecord, OnlineMember, OwnRemoval, RoleDefinition,
};
pub use store::{new_map, CommunityMap, CommunityStore};
