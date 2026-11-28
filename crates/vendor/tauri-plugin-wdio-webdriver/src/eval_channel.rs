use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::oneshot;

/// Correlates DirectEval results the webview reports back out-of-band, keyed by id.
///
/// The macOS DirectEval path runs its script via `callAsyncJavaScript` to keep the run loop pumping
/// (so the app's own `core.invoke` IPC resolves on a headless runner) but must NOT read the result
/// from that call's completion handler — macOS 26.4's WebKit reclaims it intermittently. Instead the
/// script posts `{ id, result }` to a `WKScriptMessageHandler`, which calls `complete` to wake the
/// executor awaiting `register`. Mirrors the Windows `AsyncScriptState` native-handler pattern.
/// Removable once the upstream reclaim is resolved:
/// https://github.com/webdriverio/desktop-mobile/issues/540
#[derive(Default)]
pub struct EvalResultRegistry {
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
}

impl EvalResultRegistry {
    pub fn register(&self, id: String) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        tracing::debug!(%id, "eval registered, awaiting its posted result");
        self.pending.lock().insert(id, tx);
        rx
    }

    /// No-op if `id` is unknown or already completed, so a duplicate or late report is harmless.
    pub fn complete(&self, id: &str, result: Value) {
        let sender = self.pending.lock().remove(id);
        if let Some(tx) = sender {
            if tx.send(result).is_err() {
                tracing::debug!(%id, "eval result posted after its caller stopped waiting");
            } else {
                tracing::debug!(%id, "eval result delivered");
            }
        } else {
            tracing::debug!(%id, "eval result posted for an unknown id");
        }
    }

    /// Register `id` behind a [`PendingEval`] guard, which releases the registration however the
    /// waiter ends.
    pub fn register_pending(
        self: &Arc<Self>,
        id: String,
    ) -> (PendingEval, oneshot::Receiver<Value>) {
        let rx = self.register(id.clone());
        (
            PendingEval {
                registry: Arc::clone(self),
                id,
            },
            rx,
        )
    }

    /// Drop a pending registration so its sender doesn't leak; a no-op once it was completed.
    fn release(&self, id: &str) {
        let was_pending = self.pending.lock().remove(id).is_some();
        tracing::debug!(%id, was_pending, "eval registration released");
    }
}

/// A DirectEval registration released when its waiter ends: settled, timed out, or dropped
/// mid-await by an outer cancellation (a dropped request future), so a result that never posts
/// cannot leave its sender in the registry.
pub struct PendingEval {
    registry: Arc<EvalResultRegistry>,
    id: String,
}

impl Drop for PendingEval {
    fn drop(&mut self) {
        self.registry.release(&self.id);
    }
}

#[cfg(test)]
#[path = "eval_channel_tests.rs"]
mod tests;
