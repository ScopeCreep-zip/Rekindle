//! Effect types — returned by handlers, executed by the machine loop.

use std::sync::Arc;

use tokio::sync::mpsc;

use rekindle_types::daemon::DaemonRequest;
use rekindle_types::subscription_events::SubscriptionFilter;

use super::events::{DaemonEvent, InternalEvent};
use super::state::navigation::ViewKind;

pub enum Effect {
    IpcRequest {
        request_id: u64,
        request: DaemonRequest,
    },
    IpcSubscribe {
        filters: Vec<SubscriptionFilter>,
    },
    Navigate(ViewKind),
    SetClipboard(String),
    ClearClipboard,
    OsNotify {
        title: String,
        body: String,
    },
    /// Reload the theme at runtime. The machine loop replaces its ThemeManager.
    SetTheme { name: String },
    SaveSession,
    Quit,
}

/// Immutable resources needed by the effect executor.
pub struct EffectContext {
    pub client: Arc<rekindle_client::DaemonClient>,
    pub daemon_tx: mpsc::Sender<DaemonEvent>,
    pub internal_tx: mpsc::Sender<InternalEvent>,
    pub clipboard: Option<arboard::Clipboard>,
}
