//! Our prekey record and a peer's (plan E2.4, D18): a DFLT(101) record per
//! device, owned by a keypair of its own (`OwnerKey::Prekeys`), whose
//! subkey 0 is the signed head (SPK and PQ last-resort key) and subkeys
//! 1..=100 the one-time slots. The profile's subkey 5 names it in the
//! device list. These helpers move encoded bytes; the encodings are
//! `rekindle_codec::capnp_codec::identity`'s, and the signatures are
//! checked by `rekindle_crypto` before anything is used.

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, SetOutcome};
use rekindle_types::dht_layout::prekeys::{self as layout, is_slot};
use rekindle_types::dht_layout::profile::PREKEY_RECORDS;

use crate::error::ProtocolError;

/// A peer's prekey record as read: the head and every slot it holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordRead {
    pub head: Option<Vec<u8>>,
    /// `(subkey, bytes)`, subkeys in `FIRST_SLOT..FIRST_SLOT + SLOT_COUNT`.
    pub slots: Vec<(u32, Vec<u8>)>,
}

/// Create our prekey record (a fresh owner keypair) and hold it writable.
/// Returns the lease, the record key and the owner keypair, which the
/// caller saves (keypair first, then the key; D1).
///
/// # Errors
/// The record could not be created.
pub async fn create_prekey_record(
    pool: &dyn RecordPort,
) -> Result<(LeaseId, String, String), ProtocolError> {
    let (lease, key, keypair) = pool
        .create(&rekindle_records::schema::prekeys(), None)
        .await?;
    tracing::info!(key = %key, "prekey record created");
    Ok((lease, key, keypair))
}

/// Hold our existing prekey record writable for the session.
///
/// # Errors
/// The record could not be opened within the pool's retry budget; peers
/// hold its key, so it is never re-created in its place.
pub async fn open_prekey_record(
    pool: &dyn RecordPort,
    key: &str,
    owner_keypair: &str,
) -> Result<LeaseId, ProtocolError> {
    Ok(pool.acquire(key, Some(owner_keypair)).await?)
}

/// Write the head (needs the session's writable lease). Durable: a miss is
/// re-pushed until it lands.
///
/// # Errors
/// The head is larger than a subkey holds, or the write failed outright.
pub async fn publish_head(
    pool: &dyn RecordPort,
    lease: LeaseId,
    head: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    fits("the head", &head)?;
    Ok(pool.set_durable(lease, layout::HEAD, head, None).await?)
}

/// Write one slot, `subkey` in `FIRST_SLOT..FIRST_SLOT + SLOT_COUNT`.
///
/// # Errors
/// `subkey` is not a slot, the slot is larger than a subkey holds, or the
/// write failed outright.
pub async fn publish_slot(
    pool: &dyn RecordPort,
    lease: LeaseId,
    subkey: u32,
    slot: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    if !is_slot(subkey) {
        return Err(ProtocolError::DhtError(format!(
            "subkey {subkey} is not a prekey slot"
        )));
    }
    fits("a slot", &slot)?;
    Ok(pool.set_durable(lease, subkey, slot, None).await?)
}

/// Refuse a value larger than one subkey holds (`MAX_SUBKEY_VALUE_LEN`)
/// before it reaches the network.
fn fits(what: &str, value: &[u8]) -> Result<(), ProtocolError> {
    if value.len() > layout::MAX_SUBKEY_VALUE_LEN {
        return Err(ProtocolError::DhtError(format!(
            "{what} is {} B, more than a subkey holds ({} B)",
            value.len(),
            layout::MAX_SUBKEY_VALUE_LEN
        )));
    }
    Ok(())
}

/// A peer's device list (their profile's subkey 5), read from the network
/// when `force_refresh`; `None` before they published one.
///
/// # Errors
/// The profile could not be opened or read.
pub async fn read_device_list(
    pool: &dyn RecordPort,
    profile_key: &str,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    super::profile::read_profile_subkey(pool, profile_key, PREKEY_RECORDS, force_refresh).await
}

/// A peer's prekey record: the head and every slot it holds, the subkeys
/// the network holds newer fetched from it (B2's force-refresh read).
///
/// # Errors
/// The record could not be opened or read.
pub async fn read_record(pool: &dyn RecordPort, key: &str) -> Result<RecordRead, ProtocolError> {
    let lease = pool.acquire(key, None).await?;
    let subkeys: Vec<u32> = (layout::HEAD..layout::FIRST_SLOT + layout::SLOT_COUNT).collect();
    let values = pool.read_changed(lease, &subkeys).await;
    pool.release(lease).await;
    let mut read = RecordRead::default();
    for (subkey, value) in values? {
        if subkey == layout::HEAD {
            read.head = Some(value.data);
        } else if is_slot(subkey) {
            read.slots.push((subkey, value.data));
        }
    }
    read.slots.sort_unstable_by_key(|(subkey, _)| *subkey);
    Ok(read)
}
