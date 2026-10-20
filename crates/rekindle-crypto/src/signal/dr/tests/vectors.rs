//! Known-answer vectors, computed outside this implementation (audit C-6).
//!
//! The standards' own vectors (NIST SP 800-38A F.2.5, RFC 4231 cases 1–2,
//! RFC 5869 case 1) check the primitives this crate links. The Rekindle
//! vectors were computed by a separate Python generator (`hmac`,
//! `hashlib` and the `openssl enc` CLI, no Rekindle code), which first
//! reproduced the three standards' vectors above. Its relevant lines:
//!
//! ```text
//! def hkdf(salt, ikm, info, L):
//!     if salt is None: salt = bytes(32)
//!     prk = hmac.new(salt, ikm, hashlib.sha256).digest()
//!     out=b''; t=b''; i=1
//!     while len(out)<L:
//!         t=hmac.new(prk, t+info+bytes([i]), hashlib.sha256).digest(); out+=t; i+=1
//!     return out[:L]
//! def aes_cbc(key, iv, pt, pad=True):
//!     args=['openssl','enc','-aes-256-cbc','-K',key.hex(),'-iv',iv.hex()]
//! rk=bytes(range(32)); dh=bytes(range(32,64))
//! print('kdf_rk', hkdf(rk, dh, b'rekindle-dr-root-v1', 64).hex())
//! ck=bytes(range(64,96))
//! print('kdf_ck_mk', hmac.new(ck, b'\x01', hashlib.sha256).hexdigest())
//! print('kdf_ck_next', hmac.new(ck, b'\x02', hashlib.sha256).hexdigest())
//! mk=bytes(range(96,128))
//! okm=hkdf(bytes(32), mk, b'rekindle-dr-msg-v2', 80)
//! ptxt=b'Rekindle KAT: a 1:1 message'
//! ad=bytes(range(0x80,0x80+50))
//! ct=aes_cbc(okm[:32], okm[64:80], ptxt)
//! tag=hmac.new(okm[32:64], ad+ct, hashlib.sha256).digest()[:16]
//! print('seal', (ct+tag).hex())
//! pq=bytes(1152)+b'\x42'*32
//! for n in (3,4):
//!     ikm=b'\xff'*32+b''.join(bytes([0x11*(i+1)])*32 for i in range(n))+b'\x55'*32
//!         +b'\x06'+ika+b'\x06'+ikb+b'\x09'+pq
//!     print('pqxdh%d'%n, hkdf(None, ikm, b'rekindle-pqxdh-sk-v1', 64).hex())
//! ```

use aes::Aes256;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use ed25519_dalek::VerifyingKey;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::super::kdf::{kdf_ck, kdf_rk, message_keys, open, seal};
use crate::signal::pqxdh::agreement::{kdf as pqxdh_kdf, AD_LEN};
use crate::signal::pqxdh::encode::{encode_identity, ENCODED_EC_LEN};

const KDF_RK: &str = "91070bd452a28da3f7e269df6face3ad992db9cb2d331be70cd38929bb62daae\
                      ce47b324f668f4acb7d4996cf2747170522dbabde13b14868a19c538d16eeccc";
const KDF_CK_MK: &str = "01156abd78ef59f755f1e6945e5d1e2d7ddd1f1e5fe0a164b2f5fa8ad500bfba";
const KDF_CK_NEXT: &str = "f0bdefbbad3cf097dccb03f2c87159f7608e2480543eef7741c8f2e7f226e78f";
const MSG_KEYS: &str = "101361be13b63e7a9022dbc140403c83a79bfda0df76d7438c7caac3c98150fe\
                        208145094d897f0caaef248a626f8297ede7aa2e2f06ad968803ca2a4c538460\
                        a55a705e8ad6e5cb9ea3600353a00eeb";
const SEAL: &str = "9947cb7f6f6db50f40a3b816ebd805e7fb17e79e30ba9032cc1c8948b5002743\
                    de586fb4f29072c02fb3940513159ed3";
const PQXDH3: &str = "095112685cb55f9f944e50383344a380187d30ad5e3ddd7b26a3e30f1bf263d2\
                      a2f4f6259bdb0e6cee26f6b12c0a4914b1b20fefed2bf05dde080ec4add992af";
const PQXDH4: &str = "e20ba82eb9dea6580d0bceaf14c6295d4d8ca4b59dd893956c9ca8d48aa133bf\
                      900e7e2390eb3030e120761876479969a494de45cab294da1a6557425832a41f";
/// RFC 8032 §7.1 tests 1 and 2: the identities in the PQXDH vector's AD.
const IK_A: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const IK_B: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

fn range<const N: usize>(start: u8) -> [u8; N] {
    std::array::from_fn(|i| start + u8::try_from(i).unwrap())
}

fn joined(a: &[u8], b: &[u8]) -> String {
    hex::encode([a, b].concat())
}

#[test]
fn nist_sp800_38a_f_2_5_cbc_aes256_encrypt() {
    let key =
        hex::decode("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4").unwrap();
    let pt = hex::decode(
        "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51\
         30c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
    )
    .unwrap();
    let ct = cbc::Encryptor::<Aes256>::new_from_slices(&key, &range::<16>(0))
        .unwrap()
        .encrypt_padded_vec_mut::<NoPadding>(&pt);
    assert_eq!(
        hex::encode(ct),
        "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d\
         39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b"
    );
}

#[test]
fn rfc4231_hmac_sha256_cases_1_and_2() {
    let mac = |key: &[u8], data: &[u8]| {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
        m.update(data);
        hex::encode(m.finalize().into_bytes())
    };
    assert_eq!(
        mac(&[0x0b; 20], b"Hi There"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    assert_eq!(
        mac(b"Jefe", b"what do ya want for nothing?"),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn rfc5869_case_1_hkdf_sha256() {
    let mut okm = [0u8; 42];
    Hkdf::<Sha256>::new(Some(&range::<13>(0)), &[0x0b; 22])
        .expand(&range::<10>(0xf0), &mut okm)
        .unwrap();
    assert_eq!(
        hex::encode(okm),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}

#[test]
fn kdf_rk_matches_its_vector() {
    let (root, chain) = kdf_rk(&range(0), &range(32));
    assert_eq!(joined(&*root, &*chain), KDF_RK);
}

#[test]
fn kdf_ck_matches_its_vector() {
    let (message_key, next) = kdf_ck(&range(64));
    assert_eq!(hex::encode(*message_key), KDF_CK_MK);
    assert_eq!(hex::encode(*next), KDF_CK_NEXT);
}

#[test]
fn the_message_key_expansion_matches_its_vector() {
    assert_eq!(hex::encode(*message_keys(&range(96))), MSG_KEYS);
}

#[test]
fn seal_matches_its_vector_and_opens() {
    let ad: [u8; 50] = range(0x80);
    let sealed = seal(&range(96), b"Rekindle KAT: a 1:1 message", &ad);
    assert_eq!(hex::encode(&sealed), SEAL);
    assert_eq!(
        open(&range(96), &sealed, &ad).unwrap(),
        b"Rekindle KAT: a 1:1 message"
    );
}

#[test]
fn the_pqxdh_kdf_input_layout_matches_its_vectors() {
    let identity = |hex_key: &str| {
        let bytes: [u8; 32] = hex::decode(hex_key).unwrap().try_into().unwrap();
        encode_identity(&VerifyingKey::from_bytes(&bytes).unwrap())
    };
    let mut ad = [0u8; AD_LEN];
    ad[..ENCODED_EC_LEN].copy_from_slice(&identity(IK_A));
    ad[ENCODED_EC_LEN..].copy_from_slice(&identity(IK_B));
    assert_eq!(ad[0], 0x06);
    let mut kem = vec![0x09];
    kem.extend_from_slice(&[0u8; 1152]);
    kem.extend_from_slice(&[0x42; 32]);
    let dhs: Vec<[u8; 32]> = (1..=4u8).map(|i| [0x11 * i; 32]).collect();
    for (n, expected) in [(3, PQXDH3), (4, PQXDH4)] {
        let refs: Vec<&[u8; 32]> = dhs[..n].iter().collect();
        let keys = pqxdh_kdf(&refs, &[0x55; 32], &ad, &kem).unwrap();
        assert_eq!(joined(&*keys.sk_ec, &*keys.sk_scka), expected, "{n} DHs");
    }
}
