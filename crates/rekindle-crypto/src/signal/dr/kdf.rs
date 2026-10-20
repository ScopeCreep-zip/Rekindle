//! The Double Ratchet's functions (DR rev. 4 §7.2): `KDF_RK`, `KDF_CK`,
//! `ENCRYPT` and `DECRYPT`.
//!
//! `ENCRYPT` is §7.2's concrete recipe: HKDF-SHA256 of the message key
//! (zero salt, info `DR_MSG`) gives 80 bytes, a 32-byte AES-256 key, a
//! 32-byte HMAC-SHA256 key and a 16-byte IV; the plaintext is AES-256-CBC
//! with PKCS#7 padding; the tag is HMAC-SHA256 over `AD ‖ ciphertext`,
//! truncated to 128 bits (§8.6: "acceptable", never below 64). Unlike an
//! AEAD over a counter mode, a message key used twice (a restored or
//! cloned ratchet state) leaks only whether the two plaintexts share
//! leading blocks, never their XOR or an authentication key.

use aes::Aes256;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rekindle_types::domains;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// A 32-byte key that is wiped when dropped.
pub(super) type Key = Zeroizing<[u8; 32]>;

/// The truncated HMAC tag's length (§8.6: 128 bits).
pub(super) const TAG_LEN: usize = 16;

/// The AES block, which every ciphertext is a whole number of.
const BLOCK_LEN: usize = 16;

/// The HKDF output `ENCRYPT` splits: key, MAC key, IV.
const MESSAGE_KEYS_LEN: usize = 80;

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

/// `ENCRYPT(mk, plaintext, ad)`: `AES-256-CBC(pt) ‖ HMAC(ad ‖ ct)[..16]`.
pub(super) fn seal(message_key: &[u8; 32], plaintext: &[u8], ad: &[u8]) -> Vec<u8> {
    let keys = message_keys(message_key);
    let mut out = cbc::Encryptor::<Aes256>::new_from_slices(enc_key(&keys), iv(&keys))
        .expect("a 32-byte key and a 16-byte IV")
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    let tag = tag_mac(&keys, ad, &out).finalize().into_bytes();
    out.extend_from_slice(&tag[..TAG_LEN]);
    out
}

/// `DECRYPT(mk, ciphertext, ad)`. The tag is checked in constant time
/// before anything is decrypted, and every refusal (a short or ragged
/// length, a tag that does not verify, bad padding) is the same error, so
/// a refusal says nothing about the plaintext.
pub(super) fn open(message_key: &[u8; 32], data: &[u8], ad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if data.len() < BLOCK_LEN + TAG_LEN || !(data.len() - TAG_LEN).is_multiple_of(BLOCK_LEN) {
        return Err(refused());
    }
    let (ciphertext, tag) = data.split_at(data.len() - TAG_LEN);
    let keys = message_keys(message_key);
    tag_mac(&keys, ad, ciphertext)
        .verify_truncated_left(tag)
        .map_err(|_| refused())?;
    cbc::Decryptor::<Aes256>::new_from_slices(enc_key(&keys), iv(&keys))
        .expect("a 32-byte key and a 16-byte IV")
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| refused())
}

/// HKDF-SHA256 of the message key (zero salt, info `DR_MSG`) → the 80
/// bytes `ENCRYPT` splits. A message key is used once, so the derived IV
/// is never used twice under its key.
pub(super) fn message_keys(message_key: &[u8; 32]) -> Zeroizing<[u8; MESSAGE_KEYS_LEN]> {
    let mut okm = Zeroizing::new([0u8; MESSAGE_KEYS_LEN]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), message_key)
        .expand(domains::DR_MSG.as_bytes(), &mut *okm)
        .expect("80 bytes is within HKDF-SHA256's output limit");
    okm
}

fn enc_key(keys: &[u8; MESSAGE_KEYS_LEN]) -> &[u8] {
    &keys[..32]
}

fn iv(keys: &[u8; MESSAGE_KEYS_LEN]) -> &[u8] {
    &keys[64..]
}

/// The HMAC over `ad ‖ ciphertext` under the MAC key (§7.2: "the
/// associated_data prepended to the ciphertext").
fn tag_mac(keys: &[u8; MESSAGE_KEYS_LEN], ad: &[u8], ciphertext: &[u8]) -> Hmac<Sha256> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64]).expect("HMAC takes any key length");
    mac.update(ad);
    mac.update(ciphertext);
    mac
}

fn refused() -> CryptoError {
    CryptoError::decryption("ratchet message does not authenticate".into())
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
