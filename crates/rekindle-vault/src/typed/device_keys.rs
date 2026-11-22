//! This device's per-community governance op keys (ADR 0022 rules 1 and 4,
//! plan E3.2 S7d-keys.5/.6): a random Ed25519 secret per community, minted
//! on this device, held only in the device vault (`crate::device`) under
//! `VaultKey::DeviceOpKey`, so no identity export carries it.
//!
//! Strict, unlike the best-effort community keys: a key that is not stored
//! is an endorsement no op can ever be signed under, and a key that cannot
//! be read is refused by the caller, never minted again over an endorsement.
//!
//! The same entry holds, for a device other than the member's first, the
//! record its key's endorsement binds, `M(p,d)`, and that record's owner
//! keypair `K_M(p,d)` (ADR 0022 rules 3 and 4: "the device key, `K_M(p,d)`
//! ... live in the vault under `DeviceOpKey{community}`"). The first
//! device's record is `M(p)`, owned by `K_M(p)` in the identity vault, so
//! its entry holds the key alone. Layout: the 32-byte secret, then, when a
//! record is bound, the record's length (`u16`, big-endian), the record
//! and the owner keypair text.

use zeroize::Zeroizing;

use crate::{VaultError, VaultKey, VaultStore};

fn key(community: &str) -> VaultKey {
    VaultKey::DeviceOpKey {
        community: community.to_string(),
    }
}

/// This device's op key secret for `community`, or `None` when this
/// device holds none.
///
/// # Errors
/// The vault cannot be read, or the entry is not 32 bytes.
pub fn load_device_op_key(
    device: &VaultStore,
    community: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, VaultError> {
    Ok(load_entry(device, community)?.map(|entry| entry.secret))
}

/// Store this device's op key secret for `community`. The entry then
/// holds the key alone, the first device's entry: a record an earlier
/// entry bound is gone ([`load_device_record`] is `None`).
///
/// # Errors
/// The vault cannot be written.
pub fn store_device_op_key(
    device: &VaultStore,
    community: &str,
    secret: &[u8; 32],
) -> Result<(), VaultError> {
    device.put(&key(community), secret)
}

/// The record this device's key writes its ops to, `M(p,d)`, and its owner
/// keypair `K_M(p,d)`.
pub struct DeviceRecordKeys {
    /// The record's key text.
    pub record: String,
    /// `K_M(p,d)`, the owner keypair text.
    pub owner: Zeroizing<String>,
}

/// The record bound to this device's key for `community`, `None` when the
/// entry holds the key alone (the first device, whose record is `M(p)`)
/// or no key at all.
///
/// # Errors
/// The vault cannot be read, or the entry is malformed.
pub fn load_device_record(
    device: &VaultStore,
    community: &str,
) -> Result<Option<DeviceRecordKeys>, VaultError> {
    Ok(load_entry(device, community)?.and_then(|entry| entry.record))
}

/// Bind `record` (`M(p,d)`) and its owner keypair `owner` to this device's
/// held key for `community`.
///
/// # Errors
/// No key is held for `community` (a record belongs to a key), the record
/// is longer than `u16::MAX` bytes, or the vault cannot be read or written.
pub fn store_device_record(
    device: &VaultStore,
    community: &str,
    record: &str,
    owner: &str,
) -> Result<(), VaultError> {
    let entry = load_entry(device, community)?
        .ok_or_else(|| VaultError::Schema("no device op key holds this record".into()))?;
    let len = u16::try_from(record.len())
        .map_err(|_| VaultError::Schema("device record is too long".into()))?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(34 + record.len() + owner.len()));
    bytes.extend_from_slice(entry.secret.as_slice());
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.extend_from_slice(record.as_bytes());
    bytes.extend_from_slice(owner.as_bytes());
    device.put(&key(community), &bytes)
}

struct Entry {
    secret: Zeroizing<[u8; 32]>,
    record: Option<DeviceRecordKeys>,
}

fn load_entry(device: &VaultStore, community: &str) -> Result<Option<Entry>, VaultError> {
    let Some(bytes) = device.get(&key(community))? else {
        return Ok(None);
    };
    parse_entry(&bytes).map(Some)
}

fn parse_entry(bytes: &[u8]) -> Result<Entry, VaultError> {
    let malformed = |what: &str| VaultError::Schema(format!("device op key entry: {what}"));
    let (secret, rest) = bytes
        .split_first_chunk::<32>()
        .ok_or_else(|| malformed("shorter than 32 bytes"))?;
    let secret = Zeroizing::new(*secret);
    if rest.is_empty() {
        return Ok(Entry {
            secret,
            record: None,
        });
    }
    let (len, rest) = rest
        .split_first_chunk::<2>()
        .ok_or_else(|| malformed("truncated record length"))?;
    let len = usize::from(u16::from_be_bytes(*len));
    if rest.len() <= len {
        return Err(malformed("truncated record or owner"));
    }
    let (record, owner) = rest.split_at(len);
    let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|_| malformed(what));
    Ok(Entry {
        secret,
        record: Some(DeviceRecordKeys {
            record: text(record, "record is not text")?,
            owner: Zeroizing::new(text(owner, "owner is not text")?),
        }),
    })
}

#[cfg(test)]
#[path = "device_keys_tests.rs"]
mod tests;
