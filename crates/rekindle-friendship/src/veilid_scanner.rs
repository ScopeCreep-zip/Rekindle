//! Generic adapter that bridges the Tauri/Veilid sync layer to the
//! `InboxScanner` trait without dragging `veilid-core` into this crate.
//!
//! The trait `VeilidScannerDeps` is implemented in `src-tauri` glue
//! against `Arc<AppState>` + `tauri::AppHandle`; this scanner just
//! forwards `scan()` to `deps.sync_friends_now()`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::coordinator::{InboxScanner, ScanError};

/// Dependencies the Veilid-backed inbox scanner needs. Implemented in
/// src-tauri so the friendship crate stays free of `veilid-core`,
/// `AppState`, and `tauri::AppHandle`.
#[async_trait]
pub trait VeilidScannerDeps: Send + Sync + 'static {
    /// Re-query friend DHT records (status, prekey, route) — the path
    /// through which a peer's pending friend-request state becomes
    /// visible. Returns processed count for diagnostic tracing
    /// (sync_friends_now today doesn't expose a per-friend count and
    /// returns 0).
    async fn sync_friends_now(&self) -> Result<u32, ScanError>;
}

/// Scanner that delegates to a `VeilidScannerDeps` impl. Generic over
/// the concrete deps type so the compiler monomorphises the call —
/// `Arc<dyn VeilidScannerDeps>` would work too but the plan specifies
/// the monomorphic form and a once-per-30s call has no measurable cost
/// difference either way.
pub struct VeilidInboxScanner<D: VeilidScannerDeps> {
    deps: Arc<D>,
}

impl<D: VeilidScannerDeps> VeilidInboxScanner<D> {
    #[must_use]
    pub fn new(deps: Arc<D>) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl<D: VeilidScannerDeps> InboxScanner for VeilidInboxScanner<D> {
    async fn scan(&self) -> Result<u32, ScanError> {
        self.deps.sync_friends_now().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct CountingDeps {
        calls: Mutex<u32>,
        result: Result<u32, &'static str>,
    }

    #[async_trait]
    impl VeilidScannerDeps for CountingDeps {
        async fn sync_friends_now(&self) -> Result<u32, ScanError> {
            *self.calls.lock() += 1;
            self.result
                .map_err(|e| ScanError::InboxUnavailable(e.to_string()))
        }
    }

    #[tokio::test]
    async fn scanner_forwards_ok() {
        let deps = Arc::new(CountingDeps {
            calls: Mutex::new(0),
            result: Ok(7),
        });
        let scanner = VeilidInboxScanner::new(deps.clone());
        assert_eq!(scanner.scan().await.unwrap(), 7);
        assert_eq!(*deps.calls.lock(), 1);
    }

    #[tokio::test]
    async fn scanner_forwards_err() {
        let deps = Arc::new(CountingDeps {
            calls: Mutex::new(0),
            result: Err("net down"),
        });
        let scanner = VeilidInboxScanner::new(deps.clone());
        let err = scanner.scan().await.unwrap_err();
        match err {
            ScanError::InboxUnavailable(msg) => assert_eq!(msg, "net down"),
            ScanError::Other(other) => panic!("unexpected Other variant: {other:?}"),
        }
    }
}
