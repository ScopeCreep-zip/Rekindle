//! Keybinding system — compile-time-embedded JSON keymap.

mod context;
mod map;
mod parse;
mod store;

pub use context::KeymapContext;
pub use map::KeymapAction;
pub use store::KeymapStore;
