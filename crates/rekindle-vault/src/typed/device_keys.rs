//! This device's per-community governance op keys (ADR 0022 rules 1 and 4,
//! plan E3.2 S7d-keys.5/.6): a random Ed25519 secret per community, minted
//! on this device, held only in the device vault (`crate::device`) under
//! `VaultKey::DeviceOpKey`, so no identity export carries it.
//!
//! Strict, unlike the best-effort community keys: a key that is not stored
//! is an endorsement no op can ever be signed under, and a key that cannot
//! be read is refused by the caller, never minted again over an endorsement.

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
    let Some(bytes) = device.get(&key(community))? else {
        return Ok(None);
    };
    let secret: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Schema("device op key is not 32 bytes".into()))?;
    Ok(Some(Zeroizing::new(secret)))
}

/// Store this device's op key secret for `community`.
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
