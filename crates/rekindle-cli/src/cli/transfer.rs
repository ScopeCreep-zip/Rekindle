//! `rekindle transfer …` (plan E7.2 G12): the file transfers.

use clap::Subcommand;

/// File transfers (downloads of channel attachments).
#[derive(Subcommand)]
pub enum TransferCmd {
    /// Every transfer, newest first.
    List,
    /// Resume a paused or failed download from its checkpoint, or start
    /// a channel attachment's first download to `--dest`.
    Start {
        /// The attachment id (32 hex digits).
        attachment_id: String,
        /// The community the attachment was posted in.
        #[arg(long)]
        community: String,
        /// The channel id it was posted in.
        #[arg(long)]
        channel: String,
        /// Where to save a first download (a resume keeps its own).
        #[arg(long)]
        dest: Option<std::path::PathBuf>,
    },
    /// Pause a running download; its chunks stay cached.
    Stop {
        /// The attachment id.
        attachment_id: String,
    },
    /// Print a finished download's path (never opened or executed).
    Open {
        /// The attachment id.
        attachment_id: String,
    },
    /// Forget a transfer; `--delete-file` also deletes the saved file.
    Remove {
        /// The attachment id.
        attachment_id: String,
        #[arg(long)]
        delete_file: bool,
    },
}
