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

pub(crate) fn get_signing_key(
    signing_key: &RwLock<Option<crate::state::signing_key::SigningKeyHandle>>,
) -> Option<[u8; 32]> {
    signing_key.read().as_ref().map(|h| *h.as_bytes())
}
