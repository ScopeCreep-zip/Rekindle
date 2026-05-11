//! Shared utility functions — boundary-layer primitives.
//!
//! No business logic. Paths, validation, formatting, sanitization,
//! prompts, parsing, and audit logging.

mod audit;
mod format;
mod parse;
mod paths;
mod prompt;
mod sanitize;
mod tracing_init;
mod validate;

pub use audit::audit_log;
pub use format::{
    abbreviate_key, dir_size, format_bytes, format_duration_ago, format_time_short,
    format_timestamp, format_uptime,
};
pub use parse::{
    parse_color, parse_duration_secs, parse_permissions, parse_since_timestamp, parse_u32,
};
pub use paths::{config_dir, session_path, storage_dir};
pub use prompt::{confirm, confirm_destructive, prompt_password, resolve_display_name};
pub use sanitize::sanitize_for_display;
pub use tracing_init::init_tracing;
pub use validate::{validate_display_name, validate_name};

/// Resolve a channel name or ID. Passthrough — daemon resolves names.
pub fn resolve_channel_id(channel: &str) -> String {
    channel.to_string()
}
