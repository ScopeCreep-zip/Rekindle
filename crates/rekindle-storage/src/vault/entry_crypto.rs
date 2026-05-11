//! Per-entry AES-256-GCM encrypt/decrypt.
//!
//! Every secret BLOB in the vault is encrypted with this module before
//! being stored in SQLCipher. This is the second encryption layer —
//! defense-in-depth on top of SQLCipher's page encryption.
//!
//! Wire format: `[12-byte random nonce || ciphertext || 16-byte GCM tag]`

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};

const TAG_LEN: usize = 16;
const MIN_WIRE_LEN: usize = NONCE_LEN + TAG_LEN; // 12 + 16 = 28

pub fn encrypt(key: &Zeroizing<[u8; 32]>, plaintext: &[u8]) -> StorageResult<Vec<u8>> {
    let unbound = UnboundKey::new(&AES_256_GCM, key.as_ref())
        .map_err(|e| StorageError::EntryEncrypt(format!("key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom(&mut nonce_bytes)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.to_vec();
    aead.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| StorageError::EntryEncrypt(format!("seal: {e}")))?;

    let mut wire = Vec::with_capacity(NONCE_LEN + in_out.len());
    wire.extend_from_slice(&nonce_bytes);
    wire.extend_from_slice(&in_out);
    Ok(wire)
}

pub fn decrypt(key: &Zeroizing<[u8; 32]>, wire: &[u8]) -> StorageResult<Vec<u8>> {
    if wire.len() < MIN_WIRE_LEN {
        return Err(StorageError::EntryTooShort { len: wire.len() });
    }

    let unbound = UnboundKey::new(&AES_256_GCM, key.as_ref())
        .map_err(|e| StorageError::EntryDecrypt(format!("key init: {e}")))?;
    let aead = LessSafeKey::new(unbound);

    let nonce_bytes: [u8; NONCE_LEN] = wire[..NONCE_LEN]
        .try_into()
        .expect("slice len verified above");
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = wire[NONCE_LEN..].to_vec();
    let plaintext = aead
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| StorageError::EntryDecrypt("GCM tag verification failed".into()))?;

    Ok(plaintext.to_vec())
}

// ── Passphrase-based encrypt/decrypt ─────────────────────────────────
//
// Composes Argon2id key derivation with AES-256-GCM AEAD.
// Used for identity export/import — no vault or master key required.
//
// Wire format: [16-byte salt || 12-byte nonce || ciphertext || 16-byte GCM tag]

const PASSPHRASE_SALT_LEN: usize = 16;
const ARGON2_M_COST: u32 = 65536; // 64 MB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;

/// Encrypt plaintext with a passphrase. Generates a random salt,
/// derives a 256-bit key via Argon2id, encrypts with AES-256-GCM.
/// Returns `[salt || nonce || ciphertext || tag]`.
pub fn encrypt_with_passphrase(passphrase: &[u8], plaintext: &[u8]) -> StorageResult<Vec<u8>> {
    use argon2::{Algorithm, Argon2, Params, Version};

    let mut salt = [0u8; PASSPHRASE_SALT_LEN];
    getrandom(&mut salt)?;

    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
        .map_err(|e| StorageError::PassphraseDerivation(format!("params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase, &salt, key.as_mut())
        .map_err(|e| StorageError::PassphraseDerivation(format!("argon2: {e}")))?;

    let encrypted = encrypt(&key, plaintext)?;

    let mut wire = Vec::with_capacity(PASSPHRASE_SALT_LEN + encrypted.len());
    wire.extend_from_slice(&salt);
    wire.extend_from_slice(&encrypted);
    Ok(wire)
}

/// Decrypt ciphertext that was encrypted with `encrypt_with_passphrase`.
/// Expects wire format `[16-byte salt || 12-byte nonce || ciphertext || 16-byte tag]`.
pub fn decrypt_with_passphrase(passphrase: &[u8], wire: &[u8]) -> StorageResult<Vec<u8>> {
    use argon2::{Algorithm, Argon2, Params, Version};

    const MIN_PASSPHRASE_WIRE: usize = PASSPHRASE_SALT_LEN + MIN_WIRE_LEN; // 16 + 28 = 44
    if wire.len() < MIN_PASSPHRASE_WIRE {
        return Err(StorageError::EntryTooShort { len: wire.len() });
    }

    let salt: [u8; PASSPHRASE_SALT_LEN] = wire[..PASSPHRASE_SALT_LEN]
        .try_into()
        .expect("slice len verified above");

    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
        .map_err(|e| StorageError::PassphraseDerivation(format!("params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase, &salt, key.as_mut())
        .map_err(|e| StorageError::PassphraseDerivation(format!("argon2: {e}")))?;

    decrypt(&key, &wire[PASSPHRASE_SALT_LEN..])
}

fn getrandom(buf: &mut [u8]) -> StorageResult<()> {
    aws_lc_rs::rand::SystemRandom::new()
        .fill(buf)
        .map_err(|e| StorageError::RngFailed(format!("{e}")))
}

use aws_lc_rs::rand::SecureRandom;
