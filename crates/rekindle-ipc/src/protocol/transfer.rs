//! The file transfers (plan E7.2 G12, SK2-G design §4), carried as
//! `IpcRequest::Transfer`: the Downloads view's list, and Start (resume),
//! Stop (pause), Open (reveal, never execute) and Remove of one transfer,
//! keyed by its attachment id. Every leaf keeps its own name: the variants
//! are short (`TransferRequest::List`) but each is renamed on the wire, in
//! `name()` and so in every registry (`COMMAND_TABLE`, the shell's command
//! map) to its `Transfer…` leaf name, which is unique across `IpcRequest`.

use serde::{Deserialize, Serialize};

use rekindle_types::dto::transfer::TransferTarget;

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A transfer request.
#[derive(Clone, Serialize, Deserialize)]
pub enum TransferRequest {
    /// Every transfer, newest first (`TransferRow`s).
    #[serde(rename = "TransferList")]
    List,
    /// Resume a paused or failed download from its checkpoint, or start
    /// a never-fetched attachment of `target`: without `dest` the Host
    /// answers `SkinRun::Shell(ShellOp::ChooseDownloadDest)`, the shell
    /// asks the user where to save (the native save dialog
    /// `download_attachment` uses), then sends this again with `dest`.
    /// The answer is `None` once the download has finished.
    #[serde(rename = "TransferStart")]
    Start {
        attachment_id: String,
        target: TransferTarget,
        dest: Option<String>,
    },
    /// Pause a running download; its held chunks stay cached.
    #[serde(rename = "TransferStop")]
    Stop { attachment_id: String },
    /// The finished file to reveal in the OS file manager
    /// (`TransferReveal`); never opened or executed.
    #[serde(rename = "TransferOpen")]
    Open { attachment_id: String },
    /// Forget a transfer (stopping it first); `delete_file` also deletes
    /// the saved file.
    #[serde(rename = "TransferRemove")]
    Remove {
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
            Self::List => ("TransferList", Query, Authenticated),
            Self::Start { .. } => ("TransferStart", Write, Authenticated),
            Self::Stop { .. } => ("TransferStop", Write, Authenticated),
            Self::Open { .. } => ("TransferOpen", Query, Authenticated),
            Self::Remove { .. } => ("TransferRemove", Write, Authenticated),
        }
    }

    /// No agent sends a transfer request: each names, writes, reveals or
    /// deletes a local file, which is the user's own (as
    /// `PinRequest::AttachmentDownload`).
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        match self {
            Self::List
            | Self::Start { .. }
            | Self::Stop { .. }
            | Self::Open { .. }
            | Self::Remove { .. } => None,
        }
    }

    /// The transfer's attachment id, when the request names one.
    #[must_use]
    pub fn attachment_id(&self) -> Option<&str> {
        match self {
            Self::List => None,
            Self::Start { attachment_id, .. }
            | Self::Stop { attachment_id }
            | Self::Open { attachment_id }
            | Self::Remove { attachment_id, .. } => Some(attachment_id),
        }
    }
}

impl std::fmt::Debug for TransferRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct(self.name());
        if let Some(id) = self.attachment_id() {
            d.field("attachment_id", &id);
        }
        // A destination is a local path (it names the OS user): only
        // whether one was given is logged.
        if let Self::Start { dest, .. } = self {
            d.field("dest", &dest.is_some());
        }
        if let Self::Remove { delete_file, .. } = self {
            d.field("delete_file", delete_file);
        }
        d.finish()
    }
}
