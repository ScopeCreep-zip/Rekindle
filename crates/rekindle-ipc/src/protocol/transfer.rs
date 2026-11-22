//! The file transfers (plan E7.2 G12, SK2-G design §4), carried as
//! `IpcRequest::Transfer`: the Downloads view's list, and Start (resume),
//! Stop (pause), Open (reveal, never execute) and Remove of one transfer,
//! keyed by its attachment id. Every leaf keeps its own name.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A transfer request.
#[derive(Clone, Serialize, Deserialize)]
pub enum TransferRequest {
    /// Every transfer, newest first (`TransferRow`s).
    TransferList,
    /// Resume a paused or failed download from its checkpoint.
    TransferStart { attachment_id: String },
    /// Pause a running download; its held chunks stay cached.
    TransferStop { attachment_id: String },
    /// The finished file to reveal in the OS file manager
    /// (`TransferReveal`); never opened or executed.
    TransferOpen { attachment_id: String },
    /// Forget a transfer (stopping it first); `delete_file` also deletes
    /// the saved file.
    TransferRemove {
        attachment_id: String,
        delete_file: bool,
    },
}

impl TransferRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::TransferList => ("TransferList", Query, Authenticated),
            Self::TransferStart { .. } => ("TransferStart", Write, Authenticated),
            Self::TransferStop { .. } => ("TransferStop", Write, Authenticated),
            Self::TransferOpen { .. } => ("TransferOpen", Query, Authenticated),
            Self::TransferRemove { .. } => ("TransferRemove", Write, Authenticated),
        }
    }

    /// No agent sends a transfer request: each names, writes, reveals or
    /// deletes a local file, which is the user's own (as
    /// `PinRequest::AttachmentDownload`).
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        match self {
            Self::TransferList
            | Self::TransferStart { .. }
            | Self::TransferStop { .. }
            | Self::TransferOpen { .. }
            | Self::TransferRemove { .. } => None,
        }
    }

    /// The transfer's attachment id, when the request names one.
    #[must_use]
    pub fn attachment_id(&self) -> Option<&str> {
        match self {
            Self::TransferList => None,
            Self::TransferStart { attachment_id }
            | Self::TransferStop { attachment_id }
            | Self::TransferOpen { attachment_id }
            | Self::TransferRemove { attachment_id, .. } => Some(attachment_id),
        }
    }
}

impl std::fmt::Debug for TransferRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct(self.name());
        if let Some(id) = self.attachment_id() {
            d.field("attachment_id", &id);
        }
        if let Self::TransferRemove { delete_file, .. } = self {
            d.field("delete_file", delete_file);
        }
        d.finish()
    }
}
