//! `ENCRYPT` / `DECRYPT` (DR rev. 4 §7.2, CBC + truncated HMAC) under
//! misuse (audit R3): what a reused message key does and does not leak,
//! and that every refusal looks the same.

use aes::Aes256;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::super::kdf::{message_keys, open, seal, TAG_LEN};

const MK: [u8; 32] = [0x2a; 32];
const AD: &[u8] = b"associated data";

/// A reused key (a restored or cloned state) shows that two plaintexts
/// share their leading block, and nothing after the first difference.
#[test]
fn a_reused_key_leaks_only_a_shared_leading_block() {
    let a = seal(&MK, b"0123456789abcdef: first message", AD);
    let b = seal(&MK, b"0123456789abcdef: other message", AD);
    assert_eq!(a.len(), b.len());
    assert_eq!(a[..16], b[..16]);
    assert_ne!(a[16..32], b[16..32]);
    assert_ne!(a[a.len() - TAG_LEN..], b[b.len() - TAG_LEN..]);
}

#[test]
fn neither_tag_verifies_the_others_body() {
    let a = seal(&MK, b"0123456789abcdef: first message", AD);
    let b = seal(&MK, b"0123456789abcdef: other message", AD);
    let split = a.len() - TAG_LEN;
    let a_with_b_tag = [&a[..split], &b[split..]].concat();
    let b_with_a_tag = [&b[..split], &a[split..]].concat();
    assert!(open(&MK, &a_with_b_tag, AD).is_err());
    assert!(open(&MK, &b_with_a_tag, AD).is_err());
}

#[test]
fn any_bit_flip_in_ad_ciphertext_or_tag_fails() {
    let sealed = seal(&MK, b"a message of more than one block", AD);
    for bit in 0..sealed.len() * 8 {
        let mut flipped = sealed.clone();
        flipped[bit / 8] ^= 1 << (bit % 8);
        assert!(open(&MK, &flipped, AD).is_err(), "data bit {bit}");
    }
    for bit in 0..AD.len() * 8 {
        let mut ad = AD.to_vec();
        ad[bit / 8] ^= 1 << (bit % 8);
        assert!(open(&MK, &sealed, &ad).is_err(), "ad bit {bit}");
    }
}

/// A body whose tag verifies but whose padding is wrong is refused with
/// the error a forged tag gets: no padding oracle.
#[test]
fn bad_padding_and_a_bad_mac_are_the_same_refusal() {
    let keys = message_keys(&MK);
    let mut body = cbc::Encryptor::<Aes256>::new_from_slices(&keys[..32], &keys[64..])
        .unwrap()
        .encrypt_padded_vec_mut::<NoPadding>(&[0u8; 32]);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&keys[32..64]).unwrap();
    mac.update(AD);
    mac.update(&body);
    body.extend_from_slice(&mac.finalize().into_bytes()[..TAG_LEN]);
    let bad_padding = open(&MK, &body, AD).unwrap_err();

    let mut forged = seal(&MK, b"x", AD);
    *forged.last_mut().unwrap() ^= 1;
    let bad_mac = open(&MK, &forged, AD).unwrap_err();
    assert_eq!(bad_padding.to_string(), bad_mac.to_string());
}

#[test]
fn short_or_misaligned_lengths_are_refused() {
    let sealed = seal(&MK, b"two blocks once it is padded out", AD);
    let refusal = open(&MK, &[0u8; 32], AD).unwrap_err().to_string();
    for len in (0..16 + TAG_LEN).chain([33, 47, sealed.len() - 1]) {
        let err = open(&MK, &sealed[..len.min(sealed.len())], AD).unwrap_err();
        assert_eq!(err.to_string(), refusal, "length {len}");
    }
    let mut longer = sealed.clone();
    longer.push(0);
    assert_eq!(open(&MK, &longer, AD).unwrap_err().to_string(), refusal);
}

/// The overhead is the PKCS#7 pad (1–16 bytes) and the 16-byte tag:
/// 17–32 bytes, never the 16 an AEAD would add.
#[test]
fn the_overhead_is_seventeen_to_thirty_two_bytes() {
    for len in 0..64 {
        let overhead = seal(&MK, &vec![1u8; len], AD).len() - len;
        assert!((17..=32).contains(&overhead), "{len}: {overhead}");
    }
}
