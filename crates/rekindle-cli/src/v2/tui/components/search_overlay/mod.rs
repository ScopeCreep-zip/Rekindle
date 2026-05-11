//! Search overlay — fuzzy search across communities, channels, friends, commands.

pub mod filter;
pub mod render;
pub mod state;

pub use filter::SearchItem;
pub use state::SearchOverlay;
