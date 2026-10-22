//! The data transfer objects Host calls answer with (plan E1.1): what every
//! frontend renders, in one camelCase shape, pinned by the DTO golden test.

pub mod audit;
pub mod command;
pub mod community;
pub mod dm;
pub mod friends;
pub mod group_dm;
pub mod identity;
pub mod presence;

pub use command::{CommandError, Done};
