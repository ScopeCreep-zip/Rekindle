//! Bulk transfer commands: send, status, cancel, list.
//!
//! These commands interact with the daemon's `BulkTransferRegistry`
//! via control-plane IpcRequest messages. The actual bulk data flows
//! through the lane 0x01–0x02 wire protocol, not through these commands.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::v2::output::{format, OutputMode};
use crate::v2::transport::DaemonClient;

/// Transfer subcommand variants.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum TransferCmd {
    /// Start a bulk transfer (register with the daemon).
    Start {
        /// Unique transfer identifier.
        #[arg(long)]
        transfer_id: String,
        /// Total expected bytes.
        #[arg(long)]
        total_size: u64,
        /// MIME or OCI media type.
        #[arg(long, default_value = "application/octet-stream")]
        media_type: String,
        /// Expected SHA-256 digest (sha256:hex format).
        #[arg(long)]
        digest: String,
        /// Transfer direction: "push" or "pull".
        #[arg(long, default_value = "push")]
        direction: String,
    },
    /// Query the status of a bulk transfer.
    Status {
        /// Transfer identifier to query.
        transfer_id: String,
    },
    /// Cancel an in-progress bulk transfer.
    Cancel {
        /// Transfer identifier to cancel.
        transfer_id: String,
        /// Reason for cancellation.
        #[arg(long, default_value = "user requested")]
        reason: String,
    },
    /// Mark a bulk transfer as completed.
    Complete {
        /// Transfer identifier.
        transfer_id: String,
        /// Final digest for verification.
        #[arg(long)]
        digest: String,
        /// Total bytes transferred.
        #[arg(long)]
        bytes_transferred: u64,
    },
}

/// Dispatch a transfer subcommand to the daemon.
pub async fn dispatch(
    cmd: &TransferCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let request = match cmd {
        TransferCmd::Start {
            transfer_id,
            total_size,
            media_type,
            digest,
            direction,
        } => IpcRequest::BulkTransferStart {
            transfer_id: transfer_id.clone(),
            total_size: *total_size,
            media_type: media_type.clone(),
            digest: digest.clone(),
            direction: direction.clone(),
        },
        TransferCmd::Status { transfer_id } => IpcRequest::BulkTransferStatus {
            transfer_id: transfer_id.clone(),
        },
        TransferCmd::Cancel {
            transfer_id,
            reason,
        } => IpcRequest::BulkTransferCancel {
            transfer_id: transfer_id.clone(),
            reason: reason.clone(),
        },
        TransferCmd::Complete {
            transfer_id,
            digest,
            bytes_transferred,
        } => IpcRequest::BulkTransferComplete {
            transfer_id: transfer_id.clone(),
            digest: digest.clone(),
            bytes_transferred: *bytes_transferred,
        },
    };

    let value = client.request_ok(request).await?;
    format::print_structured(&value, mode)
}
