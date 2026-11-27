//! Phase 15 — FEK (File Encryption Key) unwrap helper.
//!
//! Architecture §28.9 + plan §1.J1: chunks are encrypted once with a
//! per-file FEK; the FEK is wrapped under the channel MEK at upload
//! time. On download, the receiver reads `offer.fek_mek_generation`,
//! looks up the historical channel MEK at that generation (via the
//! `FilesDeps::historical_channel_mek` lookup cascade — keystore →
//! channel_mek_cache → mek_cache), and calls this helper to decrypt
//! `offer.wrapped_fek` into a `MediaEncryptionKey` ready for chunk
//! decryption.
//!
//! Pure function — caller supplies the resolved MEK. The 3-tier
//! cascade lookup lives in the src-tauri adapter where the
//! keystore + AppState handles are.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::attachment::AttachmentOffer;

use crate::error::FilesError;

/// Unwrap the FEK from an attachment offer using the channel MEK that
/// originally wrapped it. Returns the unwrapped FEK ready for
/// chunk-decrypt.
///
/// `mek` must be the channel MEK whose `generation` matches
/// `offer.fek_mek_generation`. Caller (or its adapter) is responsible
/// for the cascade lookup that supplies this MEK.
pub fn unwrap_fek_for_offer(
    mek: &MediaEncryptionKey,
    offer: &AttachmentOffer,
) -> Result<MediaEncryptionKey, FilesError> {
    if mek.generation() != offer.fek_mek_generation {
        return Err(FilesError::MekUnavailable {
            community: String::new(),
            generation: offer.fek_mek_generation,
        });
    }
    let raw = mek
        .decrypt(&offer.wrapped_fek)
        .map_err(|e| FilesError::Decrypt(format!("unwrap FEK: {e}")))?;
    if raw.len() != 32 {
        return Err(FilesError::Decrypt(format!(
            "wrapped FEK plaintext is {} bytes (expected 32)",
            raw.len()
        )));
    }
    let key: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| FilesError::Decrypt("FEK length mismatch".to_string()))?;
    Ok(MediaEncryptionKey::from_bytes(key, 0))
}

#[cfg(test)]
#[path = "fek_tests.rs"]
mod tests;
