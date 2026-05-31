//! Architecture §28.4 — cross-device personal sync key derivation +
//! per-subkey AES-256-GCM envelope.
//!
//! Two derivations live here:
//!
//! 1. The long-lived **sync key**, derived from the master identity
//!    secret via HKDF-SHA256 with the labels prescribed in the spec
//!    (`b"rekindle-sync-v1"` salt, `b"personal-sync-record"` info).
//!    Used to en/decrypt all four well-known subkeys of the personal
//!    DFLT record.
//!
//! 2. A **one-time pairing key**, derived from a randomly-generated
//!    pairing code + per-session salt. Used exclusively to wrap the
//!    master identity secret during device pairing (§28.4 line 3088),
//!    then discarded.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

const SYNC_SALT: &[u8] = b"rekindle-sync-v1";
const SYNC_INFO: &[u8] = b"personal-sync-record";
const PAIRING_INFO: &[u8] = b"rekindle-pairing-v1";
const NONCE_LEN: usize = 12;
pub const SYNC_KEY_LEN: usize = 32;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SyncKey([u8; SYNC_KEY_LEN]);

impl SyncKey {
    /// Derive the device-group sync key from the master identity secret.
    /// Architecture §28.4 line 3079:
    ///
    /// ```text
    /// sync_key = HKDF-SHA256(
    ///     ikm:  master_identity_private_key,
    ///     salt: b"rekindle-sync-v1",
    ///     info: b"personal-sync-record"
    /// )
    /// ```
    pub fn from_master_secret(master_secret: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(SYNC_SALT), master_secret);
        let mut out = [0u8; SYNC_KEY_LEN];
        hkdf.expand(SYNC_INFO, &mut out)
            .expect("32-byte output is a valid HKDF-SHA256 length");
        Self(out)
    }

    pub fn as_bytes(&self) -> &[u8; SYNC_KEY_LEN] {
        &self.0
    }
}

/// AES-256-GCM-encrypt `plaintext` for one subkey of the personal
/// sync record. The `subkey_index` is bound into the AAD so a
/// malicious peer can't replay one subkey's ciphertext into another.
/// Returns `nonce || ciphertext`.
pub fn encrypt_subkey(
    sync_key: &SyncKey,
    subkey_index: u32,
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(&sync_key.0).map_err(|e| format!("aes init: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let aad = subkey_aad(subkey_index);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|e| format!("encrypt: {e}"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt_subkey(
    sync_key: &SyncKey,
    subkey_index: u32,
    blob: &[u8],
) -> Result<Vec<u8>, String> {
    if blob.len() < NONCE_LEN {
        return Err("subkey blob too short for nonce".to_string());
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(&sync_key.0).map_err(|e| format!("aes init: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let aad = subkey_aad(subkey_index);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|e| format!("decrypt: {e}"))
}

fn subkey_aad(subkey_index: u32) -> [u8; 8] {
    let mut aad = [0u8; 8];
    aad[..4].copy_from_slice(b"sync");
    aad[4..].copy_from_slice(&subkey_index.to_le_bytes());
    aad
}

/// One-time pairing wrap key, derived from `(code, salt)` via
/// HKDF-SHA256. The pairing code is short, so this is *not* a
/// password-hash; the security model relies on the code being
/// transmitted out-of-band (QR scan or in-person).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PairingKey([u8; SYNC_KEY_LEN]);

impl PairingKey {
    pub fn derive(pairing_code: &str, salt: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(salt), pairing_code.as_bytes());
        let mut out = [0u8; SYNC_KEY_LEN];
        hkdf.expand(PAIRING_INFO, &mut out)
            .expect("32-byte output is a valid HKDF-SHA256 length");
        Self(out)
    }

    pub fn wrap_master_secret(
        &self,
        master_secret: &[u8],
    ) -> Result<(Vec<u8>, [u8; NONCE_LEN]), String> {
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|e| format!("aes init: {e}"))?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, master_secret)
            .map_err(|e| format!("encrypt: {e}"))?;
        Ok((ciphertext, nonce_bytes))
    }

    pub fn unwrap_master_secret(
        &self,
        ciphertext: &[u8],
        nonce_bytes: &[u8],
    ) -> Result<Vec<u8>, String> {
        if nonce_bytes.len() != NONCE_LEN {
            return Err(format!("expected {NONCE_LEN}-byte nonce"));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|e| format!("aes init: {e}"))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("decrypt: {e}"))
    }
}

/// Generate a 12-character base32 pairing code — short enough for the
/// user to type on a different device, long enough that brute-force
/// over the 60-second pairing window is infeasible (60 bits of
/// entropy).
pub fn generate_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    let mut rng = rand::rngs::OsRng;
    let mut out = String::with_capacity(12);
    rng.fill_bytes(&mut bytes);
    for b in bytes {
        out.push(ALPHABET[usize::from(b) % ALPHABET.len()] as char);
    }
    out
}

pub fn random_pairing_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_key_roundtrips_per_subkey() {
        let sync_key = SyncKey::from_master_secret(b"master-identity-secret-32-bytes!");
        let plaintext = b"hello sync";
        let blob = encrypt_subkey(&sync_key, 1, plaintext).unwrap();
        let recovered = decrypt_subkey(&sync_key, 1, &blob).unwrap();
        assert_eq!(plaintext.to_vec(), recovered);
    }

    #[test]
    fn subkey_aad_binds_index() {
        let sync_key = SyncKey::from_master_secret(b"master-identity-secret-32-bytes!");
        let blob = encrypt_subkey(&sync_key, 1, b"plaintext").unwrap();
        // Decrypting with the wrong subkey index must fail (AAD mismatch).
        let result = decrypt_subkey(&sync_key, 2, &blob);
        assert!(
            result.is_err(),
            "subkey AAD must reject cross-subkey replay"
        );
    }

    #[test]
    fn pairing_key_wraps_and_unwraps_master_secret() {
        let salt = random_pairing_salt();
        let pairing_key = PairingKey::derive("ABCDEF123456", &salt);
        let secret = [0x42u8; 32];
        let (ct, nonce) = pairing_key.wrap_master_secret(&secret).unwrap();
        let recovered = pairing_key.unwrap_master_secret(&ct, &nonce).unwrap();
        assert_eq!(secret.to_vec(), recovered);
    }

    #[test]
    fn pairing_key_rejects_wrong_code() {
        let salt = random_pairing_salt();
        let alice = PairingKey::derive("CORRECT-CODE", &salt);
        let bob = PairingKey::derive("WRONG-CODE", &salt);
        let (ct, nonce) = alice.wrap_master_secret(&[0x42u8; 32]).unwrap();
        let result = bob.unwrap_master_secret(&ct, &nonce);
        assert!(result.is_err(), "wrong pairing code must not unwrap");
    }

    #[test]
    fn pairing_codes_have_expected_shape() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 12);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
