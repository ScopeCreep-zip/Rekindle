//! Message list — core messaging component with grouping, delivery status,
//! auto-scroll, unread separator, encrypted placeholders, and reply threading.

pub mod input;
pub mod render;
pub mod state;
pub mod types;

pub use state::MessageList;
