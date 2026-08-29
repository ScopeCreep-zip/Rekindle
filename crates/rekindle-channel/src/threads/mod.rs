//! Phase 19.e-REDO — full thread pipeline.
//!
//! Ported from src-tauri/services/community/threads.rs. Crate-side
//! functions are parameterised over `D: ChannelMessagingDeps`; the
//! src-tauri facade (after 19.h-REDO) is a thin delegate that
//! constructs the adapter and calls into this module.

mod lifecycle;
mod listing;
mod messages;
mod policy;
mod support;

pub use lifecycle::{archive_thread, create_thread};
pub use listing::{list_active_threads, list_threads};
pub use messages::{load_thread_messages, send_thread_message};
pub use policy::{
    default_auto_archive_seconds, is_thread_archived, thread_member_count,
    validate_auto_archive_seconds, ThreadMessageView,
};
