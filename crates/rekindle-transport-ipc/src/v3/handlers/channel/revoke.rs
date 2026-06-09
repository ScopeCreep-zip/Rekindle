use crate::v3::codec::channel::revoke as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let revoke = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    match revoke.artifact_kind {
        0x02 => {
            // Subscription revocation — UUID in first 16 bytes of artifact_id
            let sub_id = uuid::Uuid::from_bytes(
                revoke.artifact_id[..16].try_into().unwrap()
            );
            ctx.remove_subscription(&sub_id);
        }
        0x03 => {
            // TransferId revocation — UUID in first 16 bytes
            let transfer_id = uuid::Uuid::from_bytes(
                revoke.artifact_id[..16].try_into().unwrap()
            );
            ctx.resume_registry_mut().remove(transfer_id);
        }
        0x04 => {
            // Content hash revocation — full 32-byte hash in artifact_id
            ctx.remove_content_hash(&revoke.artifact_id);
        }
        _ => {} // unknown artifact kind, ignore
    }

    Ok(())
}
