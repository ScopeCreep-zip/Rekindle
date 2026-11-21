//! The data transfer objects Host calls answer with (plan E1.1): what every
//! frontend renders, in one camelCase shape, pinned by the DTO golden test.

pub mod audit;
pub mod channel_message;
pub mod command;
pub mod community;
pub mod dm;
pub mod friends;
pub mod group_dm;
pub mod identity;
pub mod infoview;
pub mod member_groups;
pub mod onboarding;
pub mod presence;
pub mod skin;
pub mod skin_draft;
pub mod skin_layout;
pub mod skin_library;
pub mod skin_title;

pub use command::{CommandError, Done};
