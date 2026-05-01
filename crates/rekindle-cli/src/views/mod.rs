//! TUI view system — full-screen views for each major feature.
//!
//! Each view owns its components and implements the View trait for
//! draw/update/event handling. The ViewRegistry manages view transitions.
//!
//! Views are implemented in M2. This module exists so that `main.rs`
//! compiles when the `tui` feature is enabled.

// M2 will add:
// pub mod dashboard;
// pub mod channel_watch;
// pub mod dm_inbox;
// pub mod voice_session;
// pub mod friend_list;
// pub mod doctor;
// pub mod community_info;
