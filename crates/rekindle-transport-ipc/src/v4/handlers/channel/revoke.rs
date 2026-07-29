//! CHANNEL_REVOKE handler — revoke subscriptions, transfers, content hashes.
//!
//! Subscription revocation (artifact_kind 0x02) is processed immediately
//! on the Control lane which owns SubscriptionRegistry.
//!
//! TransferId revocation (0x03) and content hash revocation (0x04) are
//! forwarded to the Data lane via `revocation_tx`. The Data lane applies
//! them to ResumeRegistry and dedup caches in its select! loop.
//!
//! Revocation is a session-integrity operation. If the revocation cannot
//! reach the Data lane, the stale entry persists — an attacker who
//! captured a transfer_id before revocation can resume it after. The
//! handler returns Err(HandlerError) to terminate the session on failure.

use crate::v4::codec::channel::revoke as codec;
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::lane::data::DataRevocation;

pub fn handle(
    subscriptions: &mut SubscriptionRegistry,
    revocation_tx: &tokio::sync::mpsc::Sender<DataRevocation>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let revoke = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    match revoke.artifact_kind {
        0x02 => {
            let sub_id = uuid::Uuid::from_bytes(
                revoke.artifact_id[..16].try_into()
                    .map_err(|_| HandlerError::CodecFailed("artifact_id shorter than 16 bytes for subscription revoke".into()))?
            );
            subscriptions.remove(&sub_id);
        }
        0x03 => {
            let transfer_id = uuid::Uuid::from_bytes(
                revoke.artifact_id[..16].try_into()
                    .map_err(|_| HandlerError::CodecFailed("artifact_id shorter than 16 bytes for transfer revoke".into()))?
            );
            if revocation_tx.try_send(DataRevocation::ResumeTransfer(transfer_id)).is_err() {
                tracing::error!(%transfer_id, "revoke: revocation_tx full — resume revocation lost, session must terminate");
                return Err(HandlerError::CodecFailed("revocation channel full — resume revocation lost".into()));
            }
        }
        0x04 => {
            if revocation_tx.try_send(DataRevocation::ContentHash(revoke.artifact_id)).is_err() {
                tracing::error!("revoke: revocation_tx full — content hash revocation lost, session must terminate");
                return Err(HandlerError::CodecFailed("revocation channel full — content hash revocation lost".into()));
            }
        }
        unknown => {
            tracing::warn!(
                artifact_kind = unknown,
                "revoke: unknown artifact_kind — peer may be using a newer protocol version"
            );
        }
    }

    Ok(())
}
