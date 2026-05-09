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

fn getrandom(buf: &mut [u8]) -> StorageResult<()> {
    aws_lc_rs::rand::SystemRandom::new()
        .fill(buf)
        .map_err(|e| StorageError::RngFailed(format!("{e}")))
}

use aws_lc_rs::rand::SecureRandom;
