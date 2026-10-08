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
mod tests {
    use super::*;

    fn mek(generation: u64) -> MediaEncryptionKey {
        // Deterministic key — tests assert round-trip, not key uniqueness.
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::try_from(i).unwrap();
        }
        MediaEncryptionKey::from_bytes(bytes, generation)
    }

    fn sample_offer(wrapped_fek: Vec<u8>, generation: u64) -> AttachmentOffer {
        AttachmentOffer {
            attachment_id: [0u8; 16],
            filename: "f.bin".into(),
            mime_type: "application/octet-stream".into(),
            total_size: 0,
            chunk_count: 0,
            chunk_size: 0,
            merkle_root: [0u8; 32],
            chunk_hashes: Vec::new(),
            wrapped_fek,
            fek_mek_generation: generation,
        }
    }

    #[test]
    fn round_trips_a_freshly_wrapped_fek() {
        let channel_mek = mek(7);
        let raw_fek = [0xABu8; 32];
        let wrapped = channel_mek.encrypt(&raw_fek).unwrap();
        let offer = sample_offer(wrapped, 7);

        match unwrap_fek_for_offer(&channel_mek, &offer) {
            Ok(unwrapped) => {
                assert_eq!(unwrapped.as_bytes(), &raw_fek);
                assert_eq!(unwrapped.generation(), 0, "unwrapped FEK gen is always 0");
            }
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }

    #[test]
    fn rejects_generation_mismatch() {
        let channel_mek = mek(3);
        let wrapped = channel_mek.encrypt(&[1u8; 32]).unwrap();
        // offer claims generation 5 but caller supplies a gen-3 MEK
        let offer = sample_offer(wrapped, 5);
        match unwrap_fek_for_offer(&channel_mek, &offer) {
            Err(FilesError::MekUnavailable { generation, .. }) => assert_eq!(generation, 5),
            Ok(_) => panic!("expected MekUnavailable, got Ok"),
            Err(other) => panic!("expected MekUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn rejects_corrupt_wrapped_bytes() {
        let channel_mek = mek(1);
        // wrong key — decrypt fails authentication
        let other = mek(1);
        let wrapped = channel_mek.encrypt(&[9u8; 32]).unwrap();
        let mut tampered = wrapped.clone();
        tampered[0] ^= 0xFF;
        let offer = sample_offer(tampered, 1);
        match unwrap_fek_for_offer(&other, &offer) {
            Err(FilesError::Decrypt(_)) => {}
            Ok(_) => panic!("expected Decrypt error, got Ok"),
            Err(other) => panic!("expected Decrypt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_short_plaintext() {
        // Build a fake "wrapped FEK" whose plaintext is < 32 bytes
        let channel_mek = mek(2);
        let wrapped = channel_mek.encrypt(&[5u8; 16]).unwrap();
        let offer = sample_offer(wrapped, 2);
        match unwrap_fek_for_offer(&channel_mek, &offer) {
            Err(FilesError::Decrypt(msg)) => assert!(msg.contains("16 bytes")),
            Ok(_) => panic!("expected Decrypt error, got Ok"),
            Err(other) => panic!("expected Decrypt size mismatch, got {other:?}"),
        }
    }
}
