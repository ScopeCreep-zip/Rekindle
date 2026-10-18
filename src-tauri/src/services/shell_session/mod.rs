//! The GUI's side of the Host's unlock (plan E1.1, ADR 0020): the
//! `ShellHooks` the Host calls at its fixed points, built from the GUI's
//! stages that are not yet ported. The Host owns the lifecycle, the unlock
//! scope, the record pool, its own records, the audit step and the
//! teardown; this runs the rest inside its scope, and each Phase E port
//! deletes its part (`unlocked.rs` names the step that ports each stage).
//! F1 deletes it with the trait.
//!
//! `app` is `None` for the E2E server, which has no window and no node: the
//! loaders run, the network stages do not.

mod ended;
mod unlocked;

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use rekindle_node::host::{EndReason, OwnRecordKeys, ShellHooks, UnlockedView};

use crate::state::SharedState;

/// The GUI's `ShellHooks`.
pub struct ShellSession {
    state: SharedState,
    app: Option<tauri::AppHandle>,
}

impl ShellSession {
    #[must_use]
    pub fn new(state: SharedState, app: Option<tauri::AppHandle>) -> Arc<Self> {
        Arc::new(Self { state, app })
    }
}

#[async_trait::async_trait]
impl ShellHooks for ShellSession {
    async fn unlocked(&self, view: &UnlockedView, scope: &Arc<SessionScope>) -> Result<(), String> {
        unlocked::run(&self.state, self.app.as_ref(), view, scope).await
    }

    fn own_records(&self, keys: &OwnRecordKeys) {
        unlocked::own_records(&self.state, self.app.as_ref(), keys);
    }

    async fn ending(&self) {
        ended::ending(&self.state, self.app.as_ref()).await;
    }

    async fn ended(&self, reason: EndReason) {
        ended::ended(&self.state, self.app.as_ref(), reason).await;
    }
}
