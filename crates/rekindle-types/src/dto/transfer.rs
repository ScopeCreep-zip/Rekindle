//! A file transfer the Host runs (plan E7.2 G12, SK2-G design §4): the
//! Downloads view's rows (`TransferList`), their states and their targets.
//! A transfer is keyed by its attachment id; `target` is plan E5.1's
//! `AttachmentTarget`, of which G12 builds the channel arm (E5.1 adds
//! `Dm` and `GroupDm` to the same table).

use serde::{Deserialize, Serialize};

/// Where an attachment lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum TransferTarget {
    /// A community channel's attachment.
    Channel {
        community: String,
        channel_id: String,
    },
}

/// A transfer's state (design §4 (3)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    Queued,
    Active,
    /// Stopped by the user; its held chunks stay cached and it resumes
    /// from them.
    Paused,
    Done,
    Failed,
}

impl TransferState {
    /// The stored text (`transfers.state`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// The state stored as `text`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Queued,
            Self::Active,
            Self::Paused,
            Self::Done,
            Self::Failed,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
    }

    /// Running: Stop applies (`StopDownload`).
    #[must_use]
    pub const fn running(self) -> bool {
        matches!(self, Self::Queued | Self::Active)
    }

    /// Stopped short: Start resumes it (`StartDownload`).
    #[must_use]
    pub const fn resumable(self) -> bool {
        matches!(self, Self::Paused | Self::Failed)
    }
}

/// One transfer, as `TransferList` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRow {
    pub attachment_id: String,
    pub target: TransferTarget,
    pub state: TransferState,
    /// The checkpoint: chunks held in the cache, of `chunk_count`.
    pub chunks_done: u32,
    pub chunk_count: u32,
    pub bytes_done: u64,
    pub total: u64,
    /// The file name the transfer saves as (the offer's sanitized name).
    pub dest_name: String,
    /// The absolute path it saves to (the shell's pick).
    pub dest_path: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

/// `TransferOpen`'s answer: the finished file the shell reveals in the OS
/// file manager (`reveal_item_in_dir`). Nothing is ever opened or executed
/// (`docs/architecture/files.md`, "Show in folder"; threat model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferReveal {
    pub path: String,
}
