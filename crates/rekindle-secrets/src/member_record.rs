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

/// The opaque key Veilid gives a `VLD0` `DFLT(owner_subkeys)` record owned
/// by `owner`: BLAKE3 over the crypto kind, the owner public key and the
/// compiled schema (`storage_manager/record_key.rs:67-78`,
/// `DHTSchemaDFLT::compile` `dflt.rs:48-55`, VLD0 `generate_hash`
/// `vld0/mod.rs:332-337`). A member record's key names its owner.
#[must_use]
pub fn dflt_opaque_record_key(owner: &[u8; 32], owner_subkeys: u16) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"VLD0");
    hasher.update(owner);
    hasher.update(b"DFLT");
    hasher.update(&owner_subkeys.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// The bytes a member record's owner `K_M` signs to bind the record to a
/// pseudonym (ADR 0011 "Binding"): `MEMBER_RECORD_BINDING ‖ u32be(len) ‖
/// anchor ‖ author pseudonym ‖ u32be(len) ‖ member_record`.
fn binding_bytes(anchor: &str, author: &[u8; 32], member_record: &str) -> Vec<u8> {
    let label = rekindle_types::domains::MEMBER_RECORD_BINDING;
    let mut out = Vec::with_capacity(label.len() + 40 + anchor.len() + member_record.len());
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(
        &u32::try_from(anchor.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(anchor.as_bytes());
    out.extend_from_slice(author);
    out.extend_from_slice(
        &u32::try_from(member_record.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(member_record.as_bytes());
    out
}

/// `K_M`'s proof that `member_record` belongs to `author` in community
/// `anchor`: only the record's owner can bind it.
#[must_use]
pub fn sign_member_record_binding(
    owner_secret: &[u8; 32],
    anchor: &str,
    author: &[u8; 32],
    member_record: &str,
) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(owner_secret)
        .sign(&binding_bytes(anchor, author, member_record))
        .to_bytes()
}

/// Check `proof` against the record's owner public key.
///
/// # Errors
/// The owner key is not a valid Ed25519 key or the proof does not verify.
pub fn verify_member_record_binding(
    owner: &[u8; 32],
    anchor: &str,
    author: &[u8; 32],
    member_record: &str,
    proof: &[u8; 64],
) -> Result<(), CryptoError> {
    crate::sign::verify_signature(owner, &binding_bytes(anchor, author, member_record), proof)
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
