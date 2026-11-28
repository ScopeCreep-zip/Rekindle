//! A member's group sending key (plan E2.6): minted from the RNG port,
//! rotated to epoch + 1 on any leave or block, and the seal/open of a
//! `GroupContent` under it. The cipher is `rekindle_secrets::group_dm`'s
//! (AD `GROUP_DM_CONTENT ‖ group_id ‖ epoch ‖ sender`); the key is
//! zeroized on drop and never printed.

use rekindle_codec::capnp_codec::direct::{GroupContent, GroupId};
pub use rekindle_secrets::group_dm::{GroupSenderKey, SENDER_KEY_LEN};
use rekindle_utils::RandomSource;

use crate::error::DmError;

/// The first epoch of a member's key in a group.
pub const FIRST_EPOCH: u32 = 0;

/// A member's first key in a group.
#[must_use]
pub fn mint_first(rng: &dyn RandomSource) -> GroupSenderKey {
    GroupSenderKey::mint(FIRST_EPOCH, rng)
}

/// Our key's next epoch.
///
/// # Errors
/// [`DmError::EpochExhausted`].
pub fn rotate(key: &GroupSenderKey, rng: &dyn RandomSource) -> Result<GroupSenderKey, DmError> {
    key.next(rng).ok_or(DmError::EpochExhausted)
}

/// Seal `plaintext` (a content kind or typing) as `sender`'s entry in
/// `group_id` under `key`.
///
/// # Errors
/// [`DmError::Crypto`].
pub fn seal(
    key: &GroupSenderKey,
    group_id: GroupId,
    sender: &str,
    plaintext: &[u8],
) -> Result<GroupContent, DmError> {
    let sealed = rekindle_secrets::group_dm::seal(key, &group_id, sender, plaintext)?;
    Ok(GroupContent {
        group_id,
        epoch: key.epoch(),
        sealed,
    })
}

/// Open `sender`'s `content` under `key`, which must be of its epoch.
///
/// # Errors
/// [`DmError::StaleEpoch`]/[`DmError::KeyMissing`] for another epoch, or
/// [`DmError::Crypto`] when it does not open.
pub fn open(
    key: &GroupSenderKey,
    sender: &str,
    content: &GroupContent,
) -> Result<Vec<u8>, DmError> {
    crate::group::check_frame_epoch(sender, content.epoch, key.epoch())?;
    Ok(rekindle_secrets::group_dm::open(
        key,
        &content.group_id,
        sender,
        &content.sealed,
    )?)
}

#[cfg(test)]
#[path = "sender_key_tests.rs"]
mod tests;
