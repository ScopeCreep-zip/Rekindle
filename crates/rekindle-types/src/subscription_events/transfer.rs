//! File transfer events (plan E7.2 G12, SK2-G design §4): a transfer's
//! state changes and a finished download, which the Downloads view, the
//! skin capabilities and a pack's `OnDownloadComplete` fold.

use serde::{Deserialize, Serialize};

use crate::dto::transfer::TransferState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TransferEvent {
    /// A transfer is now in `state`.
    StateChanged {
        attachment_id: String,
        state: TransferState,
    },
    /// A download finished and its file is saved (`AppSignal::DownloadComplete`).
    DownloadComplete { attachment_id: String },
    /// A transfer was forgotten (`TransferRemove`).
    Removed { attachment_id: String },
}
