//! InboxScanner trait — abstraction for friend inbox scan logic.
//!
//! Production: `VaultInboxScanner` in `friendship/inbox.rs`.
//! Tests: `MockScanner` in `friendship/coordinator.rs` `#[cfg(test)]`.

use async_trait::async_trait;

/// Abstract inbox scanner. The coordinator calls `scan()` on every
/// non-coalesced trigger.
#[async_trait]
pub trait InboxScanner: Send + Sync + 'static {
    /// Perform one scan. Returns count of entries processed.
    /// Errors are logged by the coordinator — they do not stop it.
    async fn scan(&self) -> Result<u32, ScanError>;
}

/// Errors from the scanner.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// Inbox DHT record not reachable.
    #[error("inbox unavailable: {0}")]
    InboxUnavailable(String),

    /// Any other failure from the concrete scanner.
    #[error("scan failed: {0}")]
    Other(String),
}
