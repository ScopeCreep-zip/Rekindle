//! Server-wide constants shared across the server module.

use std::sync::Arc;

/// Rate limit: max requests per second per connection.
pub const RATE_LIMIT_MAX_TOKENS: u32 = 100;

/// Rate limit: refill interval in milliseconds.
pub const RATE_LIMIT_REFILL_MS: u64 = 1000;

/// Well-known agent name for the daemon's internal bus subscriber.
///
/// The daemon registers with this name when it connects to its own socket.
/// Requests are unicast to this connection. Other agents MUST NOT register
/// with this name — the registry enforces uniqueness.
pub const DAEMON_AGENT_NAME: &str = "daemon";

/// Pre-allocated `Arc<str>` for the daemon name. `LazyLock` initializes on
/// first dereference (one atomic pointer load thereafter). Every
/// `send_response_to` call uses `Arc::clone` (~5ns) instead of
/// `Arc::from("daemon")` (heap alloc per call).
pub static DAEMON_NAME_ARC: std::sync::LazyLock<Arc<str>> =
    std::sync::LazyLock::new(|| Arc::from(DAEMON_AGENT_NAME));
