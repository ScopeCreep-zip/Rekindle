//! Community RPC handlers and DHT inbox processor.
//!
//! Join is fully DHT-based — the owner's daemon polls the join inbox
//! record and processes pending requests by writing to the registry.
//!
//! Leave notification is best-effort RPC, fire-and-forget from the
//! leaving member. Under v2.0 it triggers the departure MEK rotation
//! and nothing else — the leaver retires its own registry slot, so no
//! recipient removes anybody. See `leave.rs`.
//!
//! Wrapped-MEK delivery arrives here too, as the one unframed inbound
//! format the transport admits. See `mek_transfer.rs`.

use parking_lot::RwLock;

mod leave;
mod mek_transfer;

pub(crate) use leave::handle_leave;
pub(crate) use mek_transfer::handle_mek_transfer;

/// Keyring label for a community's governance record owner keypair.
pub(crate) fn governance_keypair_label(governance_key: &str) -> String {
    format!(
        "community-governance-{}",
        rekindle_utils::text::prefix(governance_key, 12)
    )
}

/// Keyring label for a community's registry record owner keypair.
///
/// Both labels name credentials that `o_cnt: 0` grants no writer slot —
/// they are the record *owner* keypairs the origin flow produces, kept
/// because they are the only thing that can ever re-create a record, not
/// because they authorize a write.
pub(crate) fn registry_keypair_label(registry_key: &str) -> String {
    format!(
        "registry-{}",
        rekindle_utils::text::prefix(registry_key, 12)
    )
}

pub(crate) fn get_signing_key(
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
) -> Option<[u8; 32]> {
    signing_key.read().as_ref().map(|h| *h.as_bytes())
}
