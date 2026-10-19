//! The Double Ratchet's functions (DR rev. 1 §5.2): `KDF_RK`, `KDF_CK`,
//! `ENCRYPT` and `DECRYPT`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rekindle_types::domains;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// A 32-byte key that is wiped when dropped.
pub(super) type Key = Zeroizing<[u8; 32]>;

/// `KDF_RK(rk, dh_out)`: HKDF-SHA256 with the root key as salt and the DH
/// output as input key material → `(new root key, chain key)`.
pub(super) fn kdf_rk(root_key: &[u8; 32], dh_out: &[u8; 32]) -> (Key, Key) {
    let mut okm = Zeroizing::new([0u8; 64]);
    Hkdf::<Sha256>::new(Some(root_key), dh_out)
        .expand(domains::DR_ROOT.as_bytes(), &mut *okm)
        .expect("64 bytes is within HKDF-SHA256's output limit");
    split(&okm)
}

/// `KDF_CK(ck)`: HMAC-SHA256 keyed by the chain key over `0x01` (the
/// message key) and over `0x02` (the next chain key).
pub(super) fn kdf_ck(chain_key: &[u8; 32]) -> (Key, Key) {
    (hmac(chain_key, 0x01), hmac(chain_key, 0x02))
}

/// `ENCRYPT(mk, plaintext, ad)` with AES-256-GCM under the key and nonce
/// [`message_keys`] derives.
pub(super) fn seal(
    message_key: &[u8; 32],
    plaintext: &[u8],
    ad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let (cipher, nonce) = message_keys(message_key);
    cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: ad,
            },
        )
        .map_err(|e| CryptoError::encryption(format!("ratchet seal: {e}")))
}

/// `DECRYPT(mk, ciphertext, ad)`; fails when the tag does not verify.
pub(super) fn open(
    message_key: &[u8; 32],
    ciphertext: &[u8],
    ad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let (cipher, nonce) = message_keys(message_key);
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext,
                aad: ad,
            },
        )
        .map_err(|_| CryptoError::decryption("ratchet message does not authenticate".into()))
}

/// HKDF-SHA256 of the message key (zero salt, info `DR_MSG`) → a 32-byte
/// AES key and a 12-byte nonce. A message key is used once, so the
/// derived nonce never repeats under its key.
fn message_keys(message_key: &[u8; 32]) -> (Aes256Gcm, [u8; 12]) {
    let mut okm = Zeroizing::new([0u8; 44]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), message_key)
        .expand(domains::DR_MSG.as_bytes(), &mut *okm)
        .expect("44 bytes is within HKDF-SHA256's output limit");
    let cipher = Aes256Gcm::new_from_slice(&okm[..32]).expect("a 32-byte AES-256 key");
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&okm[32..]);
    (cipher, nonce)
}

fn hmac(key: &[u8; 32], constant: u8) -> Key {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&[constant]);
    Zeroizing::new(mac.finalize().into_bytes().into())
}

fn split(okm: &[u8; 64]) -> (Key, Key) {
    let mut first = Zeroizing::new([0u8; 32]);
    let mut second = Zeroizing::new([0u8; 32]);
    first.copy_from_slice(&okm[..32]);
    second.copy_from_slice(&okm[32..]);
    (first, second)
}
