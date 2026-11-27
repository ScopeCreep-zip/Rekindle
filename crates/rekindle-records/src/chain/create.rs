//! A participant record's creation: the record and its empty `HEAD`, on the
//! network before its key leaves.

use rekindle_types::dht_layout::dm_participant;

use super::local::local_value;
use super::{page, ChainError};
use crate::port::{OpenIntent, RecordPort};
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
pub async fn create_participant(
    port: &dyn RecordPort,
) -> Result<(String, zeroize::Zeroizing<String>), ChainError> {
    let empty_head = page::empty_head()?;
    let (lease, record_key, owner) = port.create(&schema::dm_participant(), None).await?;
    let written = port
        .set(lease, dm_participant::HEAD, empty_head, None)
        .await;
    port.release(lease).await;
    written?.require_stored(dm_participant::HEAD)?;
    Ok((record_key, owner))
}

/// Put our dm participant record `record_key` (created locally, its key
/// stored) on the network before its key is sent: its empty `HEAD` is
/// written, and must land, unless our copy already holds a `HEAD`, which
/// only a landed write leaves (an append's `HEAD` write that missed stores
/// nothing: `AllowOffline(false)`, 0.5.8 `local_record_store_interface.rs:60-62`).
/// The record is made locally at the first frame so the first hints never
/// wait on the network (they carry their frames); the key's
/// `StreamRecord` waits on this, and is attempted again while it fails.
/// `writer` is the record's owner keypair.
///
/// # Errors
/// The record could not be opened, or the `HEAD` write did not land.
pub async fn publish_participant(
    port: &dyn RecordPort,
    record_key: &str,
    writer: &str,
) -> Result<(), ChainError> {
    let empty_head = page::empty_head()?;
    let lease = port
        .acquire(record_key, Some(writer), OpenIntent::Hold)
        .await?;
    let written = match local_value(port, lease, dm_participant::HEAD).await {
        Ok(Some(_)) => Ok(None),
        Ok(None) => port
            .set(lease, dm_participant::HEAD, empty_head, None)
            .await
            .map(Some),
        Err(e) => Err(e),
    };
    port.release(lease).await;
    if let Some(outcome) = written? {
        outcome.require_stored(dm_participant::HEAD)?;
    }
    Ok(())
}
