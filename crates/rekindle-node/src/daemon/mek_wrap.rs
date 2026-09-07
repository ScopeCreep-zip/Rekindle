//! Wrapping a fresh MEK for a community's members.
//!
//! The same `filter_map` appeared verbatim in three rekey paths —
//! `community_rpc::leave`, `community_rpc::inbox_stages` and
//! `governance_rpc::rekey`. Each decoded every member's pseudonym hex,
//! wrapped the key for them, and silently skipped anyone whose hex did
//! not parse. Three copies of a "silently skip" rule is how one of them
//! ends up skipping for a different reason than the others.

use rekindle_transport::payload::dht_types::{EncryptedMekCopy, MemberSummary};

/// Wrap `mek_wire` for every member whose pseudonym parses.
///
/// A member is skipped when their `pseudonym_key` is not 32 bytes of
/// valid hex, or when wrapping fails for them — the rotation still
/// proceeds for everyone else, because one unparseable member must not
/// block the whole community's rekey.
pub(crate) fn wrap_for_members(
    rotator: &ed25519_dalek::SigningKey,
    members: &[MemberSummary],
    mek_wire: &[u8],
) -> Vec<EncryptedMekCopy> {
    members
        .iter()
        .filter_map(|m| {
            let pub_bytes: [u8; 32] = hex::decode(&m.pseudonym_key).ok()?.try_into().ok()?;
            rekindle_transport::crypto::mek::wrap_mek(rotator, &pub_bytes, mek_wire)
                .ok()
                .map(|wrapped| EncryptedMekCopy {
                    target_pseudonym: m.pseudonym_key.clone(),
                    encrypted_mek: wrapped,
                })
        })
        .collect()
}
