//! Governance retirement metadata on a chain head. It leaves the old log
//! reachable and does not consume a sequence number in that device's chain.

use rekindle_codec::capnp_codec::chain as wire;
use rekindle_types::dht_layout::member_record::GOV_HEAD;

use super::{page, read_unretired, ChainError, ChainHead, ChainRead, HEAD_ROUNDS};
use crate::port::{RecordPort, SetOutcome};

pub use wire::DeviceRetirementPointer;
pub(super) type HeadValue = (Option<ChainHead>, Option<DeviceRetirementPointer>);

pub(super) async fn read_head_value(
    port: &dyn RecordPort,
    record: &str,
    subkey: u32,
) -> Result<HeadValue, ChainError> {
    let Some(value) = port.read_once(record, subkey, true).await? else {
        return Ok((None, None));
    };
    let (head, retirement) = wire::decode_head_value(&value.data)?;
    if let Some(head) = &head {
        page::check_head(head)?;
    }
    Ok((head, retirement))
}

/// Read GOV_HEAD once for both the old op log and its retirement locator.
///
/// # Errors
/// The head or a page could not be read or decoded.
pub async fn read_log_with_retirement(
    port: &dyn RecordPort,
    record: &str,
    cursor: u64,
) -> Result<(ChainRead, Option<DeviceRetirementPointer>), ChainError> {
    let (head, retirement) = read_head_value(port, record, GOV_HEAD).await?;
    let read = read_unretired(port, head.map(|h| h.newest), cursor).await?;
    Ok((read, retirement))
}

/// Publish a retirement locator on the old device's GOV_HEAD using its
/// record owner, after the successor has landed the named op.
///
/// # Errors
/// The head could not be read, decoded or written.
pub async fn publish_device_retirement(
    port: &dyn RecordPort,
    record: &str,
    writer: &str,
    pointer: &DeviceRetirementPointer,
) -> Result<(), ChainError> {
    let mut round = 1;
    loop {
        let (head, _) = read_head_value(port, record, GOV_HEAD).await?;
        let bytes = page::encode_head_value(head.as_ref(), Some(pointer))?;
        let outcome = port
            .write_own_once(record, GOV_HEAD, bytes, Some(writer))
            .await?;
        if matches!(outcome, SetOutcome::Superseded(_)) && round < HEAD_ROUNDS {
            round += 1;
            continue;
        }
        return Ok(outcome.require_stored(GOV_HEAD)?);
    }
}
