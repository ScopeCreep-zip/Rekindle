//! Key derivation functions for the Double Ratchet.
//!
//! - `KDF_RK`: root-key ratchet step → (new_rk, new_ck). 64-byte HKDF output.
//! - `KDF_RK_HE`: header-encrypted variant → (new_rk, new_ck, new_hk). 96-byte HKDF output.
//! - `KDF_CK`: chain-key step → (next_ck, message_key). HMAC-SHA-256 per Signal spec §5.2.
//! - `KDF_HYBRID`: Triple Ratchet combiner → mixed AEAD key.

use aws_lc_rs::{hkdf, hmac};
use zeroize::Zeroizing;

use crate::error::RatchetError;

const INFO_KDF_RK: &[u8] = b"Rekindle_DR_RK_v1";
const INFO_KDF_RK_HE: &[u8] = b"Rekindle_DR_RK_HE_v1";
const INFO_KDF_HYBRID: &[u8] = b"Rekindle_TR_HYBRID_v1";
const INFO_KDF_DRONLY: &[u8] = b"Rekindle_TR_DRONLY_v1";

// HKDF algorithm: SHA-256 in both SOTA and FIPS modes for DR KDFs.
// The crypto spec §8.3 confirms KDF_RK uses HKDF-SHA-256 in both columns.
const DR_HKDF: hkdf::Algorithm = hkdf::HKDF_SHA256;

/// HKDF output length adapter. `hkdf::KeyType` is an open trait in aws-lc-rs.
#[derive(Clone, Copy)]
struct OkmLen<const N: usize>;
impl<const N: usize> hkdf::KeyType for OkmLen<N> {
    fn len(&self) -> usize {
        N
    }
}

/// Output of `kdf_rk_he`: `(new_root_key, new_chain_key, next_header_key)`.
pub type RkHeOutput = (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>);

/// Root-key ratchet step: `(rk, dh_out) → (new_rk, new_ck)`.
pub fn kdf_rk(
    rk: &Zeroizing<[u8; 32]>,
    dh_out: &Zeroizing<[u8; 32]>,
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), RatchetError> {
    let salt = hkdf::Salt::new(DR_HKDF, rk.as_ref());
    let prk = salt.extract(dh_out.as_ref());
    let okm = prk.expand(&[INFO_KDF_RK], OkmLen::<64>).map_err(|_| RatchetError::Kdf)?;
    let mut buf = [0u8; 64];
    okm.fill(&mut buf).map_err(|_| RatchetError::Kdf)?;
    let mut new_rk = Zeroizing::new([0u8; 32]);
    let mut new_ck = Zeroizing::new([0u8; 32]);
    new_rk.copy_from_slice(&buf[..32]);
    new_ck.copy_from_slice(&buf[32..]);
    buf.fill(0);
    Ok((new_rk, new_ck))
}

/// Header-encrypted root-key step: `(rk, dh_out) → (new_rk, new_ck, next_hk)`.
pub fn kdf_rk_he(
    rk: &Zeroizing<[u8; 32]>,
    dh_out: &Zeroizing<[u8; 32]>,
) -> Result<RkHeOutput, RatchetError> {
    let salt = hkdf::Salt::new(DR_HKDF, rk.as_ref());
    let prk = salt.extract(dh_out.as_ref());
    let okm = prk
        .expand(&[INFO_KDF_RK_HE], OkmLen::<96>)
        .map_err(|_| RatchetError::Kdf)?;
    let mut buf = [0u8; 96];
    okm.fill(&mut buf).map_err(|_| RatchetError::Kdf)?;
    let mut new_rk = Zeroizing::new([0u8; 32]);
    let mut new_ck = Zeroizing::new([0u8; 32]);
    let mut nhk = Zeroizing::new([0u8; 32]);
    new_rk.copy_from_slice(&buf[..32]);
    new_ck.copy_from_slice(&buf[32..64]);
    nhk.copy_from_slice(&buf[64..]);
    buf.fill(0);
    Ok((new_rk, new_ck, nhk))
}

/// Chain-key step: `ck → (next_ck, message_key)`.
///
/// Always HMAC-SHA-256, both SOTA and FIPS. Signal DR spec §5.2 fixes this.
pub fn kdf_ck(
    ck: &Zeroizing<[u8; 32]>,
) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let key = hmac::Key::new(hmac::HMAC_SHA256, ck.as_ref());
    let ck_tag = hmac::sign(&key, &[0x02]);
    let mk_tag = hmac::sign(&key, &[0x01]);
    let mut next_ck = Zeroizing::new([0u8; 32]);
    let mut mk = Zeroizing::new([0u8; 32]);
    next_ck.copy_from_slice(&ck_tag.as_ref()[..32]);
    mk.copy_from_slice(&mk_tag.as_ref()[..32]);
    (next_ck, mk)
}

/// Triple Ratchet hybrid combiner: `(ec_mk, pq_mk) → mixed AEAD key`.
///
/// Per Signal DR spec §7.2: `HKDF(salt=pq_mk, ikm=ec_mk, info=HYBRID)`.
pub fn kdf_hybrid(
    ec_mk: &Zeroizing<[u8; 32]>,
    pq_mk: &Zeroizing<[u8; 32]>,
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let salt = hkdf::Salt::new(DR_HKDF, pq_mk.as_ref());
    let prk = salt.extract(ec_mk.as_ref());
    let okm = prk
        .expand(&[INFO_KDF_HYBRID], OkmLen::<32>)
        .map_err(|_| RatchetError::Kdf)?;
    let mut out = Zeroizing::new([0u8; 32]);
    okm.fill(out.as_mut()).map_err(|_| RatchetError::Kdf)?;
    Ok(out)
}

/// DR-only combiner (before first SPQR epoch completes): `ec_mk → AEAD key`.
pub fn kdf_hybrid_dronly(
    ec_mk: &Zeroizing<[u8; 32]>,
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let salt = hkdf::Salt::new(DR_HKDF, &[0u8; 32]);
    let prk = salt.extract(ec_mk.as_ref());
    let okm = prk
        .expand(&[INFO_KDF_DRONLY], OkmLen::<32>)
        .map_err(|_| RatchetError::Kdf)?;
    let mut out = Zeroizing::new([0u8; 32]);
    okm.fill(out.as_mut()).map_err(|_| RatchetError::Kdf)?;
    Ok(out)
}
