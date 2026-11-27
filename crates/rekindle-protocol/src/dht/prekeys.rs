//! Our prekey record and a peer's (plan E2.4, D18): a DFLT(101) record per
//! device, owned by a keypair of its own (`OwnerKey::Prekeys`), whose
//! subkey 0 is the signed head (SPK and PQ last-resort key) and subkeys
//! 1..=100 the one-time slots. The profile's subkey 5 names it in the
//! device list. These helpers move encoded bytes; the encodings are
//! `rekindle_codec::capnp_codec::identity`'s, and the signatures are
//! checked by `rekindle_crypto` before anything is used.

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{OpenIntent, RecordPort, ReportScope, SetOutcome};
use rekindle_types::dht_layout::prekeys::{self as layout, is_slot};
use rekindle_types::dht_layout::profile::PREKEY_RECORDS;

use crate::error::ProtocolError;

/// Our own prekey record as read back: the head and every slot it holds.
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
    Ok(pool
        .acquire(key, Some(owner_keypair), OpenIntent::Hold)
        .await?)
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

/// Write the head or one slot once, not held for re-push: a miss is the
/// caller's to handle (the identity rotation, which must not leave a held
/// value of a set it abandons to land later).
///
/// # Errors
/// `subkey` is neither the head nor a slot, the value is larger than a
/// subkey holds, or the write failed outright.
pub async fn write_once(
    pool: &dyn RecordPort,
    lease: LeaseId,
    subkey: u32,
    value: Vec<u8>,
) -> Result<SetOutcome, ProtocolError> {
    if subkey != layout::HEAD && !is_slot(subkey) {
        return Err(ProtocolError::DhtError(format!(
            "subkey {subkey} is not the prekey head or a slot"
        )));
    }
    fits("a prekey value", &value)?;
    Ok(pool.set(lease, subkey, value, None).await?)
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

/// The record's head followed by every one-time slot.
fn record_subkeys() -> Vec<u32> {
    std::iter::once(layout::HEAD)
        .chain(layout::FIRST_SLOT..layout::SUBKEY_COUNT)
        .collect()
}

/// Our own prekey record read back: the head and every slot it holds, the
/// subkeys the network holds newer fetched from it (B1's reconcile). A
/// peer's record is read with [`PeerRecord`], one slot at a time.
///
/// # Errors
/// The record could not be opened or read.
pub async fn read_record(pool: &dyn RecordPort, key: &str) -> Result<RecordRead, ProtocolError> {
    let lease = pool.acquire(key, None, OpenIntent::Check).await?;
    let subkeys = record_subkeys();
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

/// A peer's prekey record opened for a first contact (plan E2.4 B2, R1-8):
/// one inspect over the head and every slot, then the head and **one**
/// slot read, never the whole record. A first contact holds none of it
/// locally, so each subkey read is a network get; reading all 101 cost
/// ten waves of gets before the PreKeyInit could go. Signal's bundle
/// carries one one-time prekey (PQXDH §3.3); the caller draws the slot
/// among [`present_slots`](Self::present_slots) and reads another only
/// when the drawn one does not decode, verify or is one it used.
///
/// Every read plans from the one inspect, as `read_changed` does: a
/// subkey the network holds newer is fetched from it, one whose local
/// copy is current is read locally. [`close`](Self::close) returns the
/// borrow.
pub struct PeerRecord<'a> {
    pool: &'a dyn RecordPort,
    lease: LeaseId,
    /// `(subkey, network holds newer)` of each subkey that holds a value.
    present: Vec<(u32, bool)>,
}

impl<'a> PeerRecord<'a> {
    /// Borrow the record at `key` and inspect its head and slots once.
    ///
    /// # Errors
    /// The record could not be opened, or no node answered the inspect.
    pub async fn open(pool: &'a dyn RecordPort, key: &str) -> Result<Self, ProtocolError> {
        let lease = pool.acquire(key, None, OpenIntent::Check).await?;
        let subkeys = record_subkeys();
        let report = match pool
            .inspect(lease, Some(&subkeys), ReportScope::UpdateGet)
            .await
        {
            Ok(report) => report,
            Err(e) => {
                pool.release(lease).await;
                return Err(e.into());
            }
        };
        let present: Vec<(u32, bool)> = report
            .subkeys
            .iter()
            .zip(report.local_seqs.iter().zip(&report.network_seqs))
            .filter(|(_, (local, network))| local.is_some() || network.is_some())
            .map(|(subkey, (local, network))| (*subkey, network > local))
            .collect();
        tracing::debug!(key = %key, present = present.len(), "peer prekey record inspected");
        Ok(Self {
            pool,
            lease,
            present,
        })
    }

    /// The slots the inspect shows holding a value, in subkey order.
    #[must_use]
    pub fn present_slots(&self) -> Vec<u32> {
        self.present
            .iter()
            .map(|(subkey, _)| *subkey)
            .filter(|subkey| is_slot(*subkey))
            .collect()
    }

    /// The head; `None` when the inspect shows none.
    ///
    /// # Errors
    /// The read failed.
    pub async fn head(&self) -> Result<Option<Vec<u8>>, ProtocolError> {
        self.read(layout::HEAD).await
    }

    /// One slot, `subkey` among [`present_slots`](Self::present_slots);
    /// `None` for any other subkey.
    ///
    /// # Errors
    /// The read failed.
    pub async fn slot(&self, subkey: u32) -> Result<Option<Vec<u8>>, ProtocolError> {
        if !is_slot(subkey) {
            return Ok(None);
        }
        self.read(subkey).await
    }

    async fn read(&self, subkey: u32) -> Result<Option<Vec<u8>>, ProtocolError> {
        let Some(&(_, newer)) = self.present.iter().find(|(s, _)| *s == subkey) else {
            return Ok(None);
        };
        let value = self.pool.get(self.lease, subkey, newer).await?;
        tracing::debug!(
            subkey,
            from_network = newer,
            found = value.is_some(),
            "peer prekey subkey read"
        );
        Ok(value.map(|v| v.data))
    }

    /// Return the borrow.
    pub async fn close(self) {
        self.pool.release(self.lease).await;
    }
}
