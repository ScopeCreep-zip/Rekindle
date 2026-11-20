//! A participant record's creation: the record and its empty `HEAD`, on the
//! network before its key leaves.

use rekindle_types::dht_layout::dm_participant;

use super::{page, ChainError};
use crate::port::RecordPort;
use crate::schema;

/// Create a participant record (`dht_layout::dm_participant`) and write its
/// empty `HEAD` once, so the record is on the network before its key
/// leaves: Veilid creates a record locally and writes nothing to the network
/// (0.5.8 `storage_manager/create_record.rs`), so a reader handed the key of
/// an unwritten record finds no record (`KeyNotFound`) until the first
/// append, as a friend invite's request inbox is opened with `OPENED`
/// before its key leaves (`dht_layout::request_inbox`). Returns the record
/// key and its owner keypair; the caller saves the keypair.
///
/// # Errors
/// The create failed, or the `HEAD` write did not land.
pub async fn create_participant(port: &dyn RecordPort) -> Result<(String, String), ChainError> {
    let (lease, record_key, owner) = port.create(&schema::dm_participant(), None).await?;
    let written = port
        .set(lease, dm_participant::HEAD, page::empty_head(), None)
        .await;
    port.release(lease).await;
    written?.require_stored(dm_participant::HEAD)?;
    Ok((record_key, owner))
}
