//! A sealed frame's own checks (plan E2.3), before anything acts on it:
//! its signature names us and verifies under the sender, it opens, and it
//! is fresh. [`super::frame`] adds the replay guard.

use rekindle_codec::capnp_codec::direct::{DmTypeId, SealedBody, SealedContent};
use rekindle_crypto::{presession, Identity};
use rekindle_types::message::{ENVELOPE_FRESHNESS_WINDOW_MS, ENVELOPE_MAX_FUTURE_SKEW_MS};

use crate::operations::dm::frame::sealed_info;

/// The content of a sealed frame from `sender` to `identity` at `now_ms`:
/// its signature names us as the recipient and verifies under the sender,
/// it opens, and it is fresh.
///
/// # Errors
/// Why it was refused.
pub fn open_presession(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
    now_ms: u64,
) -> Result<SealedContent, &'static str> {
    verify_presession(identity, sender, type_id, sealed)?;
    open_verified(identity, sender, type_id, sealed, now_ms)
}

fn signed_seal(sealed: &SealedBody) -> presession::SignedSeal {
    presession::SignedSeal {
        enc: sealed.enc,
        ciphertext: sealed.ciphertext.clone(),
        signature: sealed.signature,
    }
}

/// Whether a sealed frame of a sealed kind is signed by `sender` for us
/// (`rekindle_crypto::presession`, before anything is decrypted).
pub(super) fn verify_presession(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
) -> Result<(), &'static str> {
    sealed_info(type_id).ok_or("not a sealed kind")?;
    presession::verify(
        &identity.public_key_bytes(),
        sender,
        type_id.as_u16(),
        &signed_seal(sealed),
    )
    .map_err(presession::Refused::as_str)
}

/// The fresh content of a sealed frame whose signature verified.
pub(super) fn open_verified(
    identity: &Identity,
    sender: &[u8; 32],
    type_id: DmTypeId,
    sealed: &SealedBody,
    now_ms: u64,
) -> Result<SealedContent, &'static str> {
    let info = sealed_info(type_id).ok_or("not a sealed kind")?;
    let plaintext = presession::open(
        identity,
        sender,
        type_id.as_u16(),
        info,
        &signed_seal(sealed),
    )
    .map_err(presession::Refused::as_str)?;
    let content = SealedContent::decode(&plaintext).map_err(|_| "content does not decode")?;
    if !fresh(content.sent_at_ms, now_ms) {
        return Err("stale");
    }
    Ok(content)
}

/// Whether a frame sent at `sent_at` is inside the freshness window at
/// `now`, allowing the sender's clock to run a little ahead.
fn fresh(sent_at: u64, now: u64) -> bool {
    sent_at <= now.saturating_add(ENVELOPE_MAX_FUTURE_SKEW_MS)
        && now.saturating_sub(sent_at) <= ENVELOPE_FRESHNESS_WINDOW_MS
}

#[cfg(test)]
#[path = "sealed_open_tests.rs"]
mod tests;
