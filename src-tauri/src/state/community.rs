//! A joined community's state and its channels, categories and roles: the
//! Host's one model (`rekindle_node::host::community`, plan E3.1), which the
//! desktop's readers keep reaching through `crate::state`.

pub use rekindle_node::host::community::{
    display_role_name, CategoryInfo, ChannelInfo, ChannelType, CommunityMap, CommunityRecords,
    CommunityState, MemberProfileSnapshot, RoleDefinition,
};
