//! Channel tree — sidebar navigation for communities and channels.

pub mod input;
pub mod render;
pub mod tree;
pub mod types;

pub use tree::ChannelTree;
pub use types::{ChannelEntry, TreeNodeId};
