//! The AEAD of a member's own record fields (ADR 0011, plan E3.2): the
//! history ad (subkey 5) and the beat's session extras (subkey 1) are
//! sealed under the community text key of the current
//! `KeyScope::Community` generation, bound to the community and the member
//! record they are written in. A removed member keeps its record key but
//! not the key minted by the departure rotation, so it cannot open what is
//! sealed after it (E3.5).
//!
//! Output is `nonce(12) ‖ ciphertext+tag`, AES-256-GCM, as every MEK seal.

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use rekindle_types::error::CryptoError;
use rekindle_utils::random::RandomSource;

use crate::keys::MediaEncryptionKey;

/// The record a sealed field is written in: its AAD,
/// `u32be(len) ‖ community ‖ u32be(len) ‖ member_record`.
#[derive(Debug, Clone, Copy)]
pub struct MemberRecordAad<'a> {
    pub community_id: &'a str,
    pub member_record: &'a str,
}

impl MemberRecordAad<'_> {
    fn to_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.community_id.len() + self.member_record.len());
        for part in [self.community_id, self.member_record] {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
            out.extend_from_slice(part.as_bytes());
        }
        out
    }
}

/// Seal `plaintext` under `key` for the record `at`, with a nonce from
/// `rng`, the engine's injected generator (plan C9.2).
pub fn seal_member_record_field(
    key: &MediaEncryptionKey,
    at: MemberRecordAad<'_>,
    plaintext: &[u8],
    rng: &dyn RandomSource,
) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut nonce = [0u8; 12];
    rng.fill(&mut nonce);
    let aad = at.to_bytes();
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|e| CryptoError::Encryption(e.to_string()))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Open a field sealed by [`seal_member_record_field`] for the same `at`.
pub fn open_member_record_field(
    key: &MediaEncryptionKey,
    at: MemberRecordAad<'_>,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < 12 + 16 {
        return Err(CryptoError::Decryption("sealed field too short".into()));
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::Decryption(e.to_string()))?;
    let aad = at.to_bytes();
    cipher
        .decrypt(
            Nonce::from_slice(&sealed[..12]),
            Payload {
                msg: &sealed[12..],
                aad: &aad,
            },
        )
        .map_err(|e| CryptoError::Decryption(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_utils::random::OsRandom;

    const AT: MemberRecordAad<'static> = MemberRecordAad {
        community_id: "community",
        member_record: "VLD0:member",
    };

    #[test]
    fn round_trip_and_binding() {
        let key = MediaEncryptionKey::from_bytes([7u8; 32], 3);
        let sealed = seal_member_record_field(&key, AT, b"ranges", &OsRandom).unwrap();
        assert_eq!(
            open_member_record_field(&key, AT, &sealed).unwrap(),
            b"ranges"
        );
        let other_record = MemberRecordAad {
            member_record: "VLD0:other",
            ..AT
        };
        assert!(open_member_record_field(&key, other_record, &sealed).is_err());
        // The boundary between the two parts is part of the AAD.
        let shifted = MemberRecordAad {
            community_id: "communityV",
            member_record: "LD0:member",
        };
        assert!(open_member_record_field(&key, shifted, &sealed).is_err());
        let rotated = MediaEncryptionKey::from_bytes([8u8; 32], 4);
        assert!(open_member_record_field(&rotated, AT, &sealed).is_err());
    }
}
