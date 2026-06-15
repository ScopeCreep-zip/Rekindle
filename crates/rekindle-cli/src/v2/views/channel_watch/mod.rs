//! Channel watch view — three-pane live message stream with split-pane DM.

pub mod events;
pub mod input;
pub mod layout;
pub mod split_dm;
pub mod state;
pub mod thread_panel;

pub use state::ChannelWatchView;
