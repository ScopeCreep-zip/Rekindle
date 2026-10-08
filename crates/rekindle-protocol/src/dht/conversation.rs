use super::parse_record_key;
use super::pool::RecordPool;
use crate::error::ProtocolError;
use rekindle_codec::capnp_codec::conversation::{decode_conversation_header, ConversationHeader};
use rekindle_crypto::DhtRecordKey;

/// Read and decrypt a peer's per-contact conversation header, or `None`
/// when the record holds none.
///
/// The conversation record is read-only to its one reader
/// (`sync_service`). The write half (`ConversationRecord::{create,
/// open_write, write_header, watch, close}`) had no caller: nothing in the
/// product writes the record (C5.3 finding 6), and E2.5 deletes the whole
/// path in favour of the DM records. It was not carried onto the record
/// pool (plan C7.4).
///
/// # Errors
/// The record could not be opened or read, or its header does not decrypt
/// or decode.
pub async fn read_conversation_header(
    pool: &RecordPool,
    key: &str,
    encryption_key: &DhtRecordKey,
) -> Result<Option<ConversationHeader>, ProtocolError> {
    let lease = pool.acquire(&parse_record_key(key)?, None).await?;
    let value = pool.get(lease, 0, false).await;
    pool.release(lease).await;
    match value? {
        Some(v) => {
            let plaintext = encryption_key.decrypt(v.data())?;
            Ok(Some(decode_conversation_header(&plaintext)?))
        }
        None => Ok(None),
    }
}
