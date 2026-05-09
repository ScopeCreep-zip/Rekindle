//! ML-KEM-768 operations via `aws_lc_rs::kem`.
//!
//! ML-KEM-768 sizes: ek=1184B, dk=2400B, ct=1088B, ss=32B.
//! These are compile-time constants — any deviation is a bug.

use aws_lc_rs::kem::{DecapsulationKey, ML_KEM_768};
use zeroize::Zeroizing;

use crate::error::RatchetError;

pub const EK_LEN: usize = 1184;
pub const DK_LEN: usize = 2400;
pub const CT_LEN: usize = 1088;
pub const SS_LEN: usize = 32;

/// ML-KEM-768 keypair material returned by [`keygen`].
pub struct MlKemKeyMaterial {
    /// Decapsulation key (private). 2400 bytes, zeroized on drop.
    pub dk_bytes: Zeroizing<[u8; DK_LEN]>,
    /// Encapsulation key (public). 1184 bytes.
    pub ek_bytes: [u8; EK_LEN],
}

/// Generate an ML-KEM-768 keypair.
pub fn keygen() -> Result<MlKemKeyMaterial, RatchetError> {
    let dk = DecapsulationKey::generate(&ML_KEM_768).map_err(|_| RatchetError::KemKeygen)?;

    let ek = dk.encapsulation_key().map_err(|_| RatchetError::KemKeygen)?;
    let ek_raw = ek.key_bytes().map_err(|_| RatchetError::KemKeygen)?;
    let ek_slice = ek_raw.as_ref();
    if ek_slice.len() != EK_LEN {
        return Err(RatchetError::KemKeygen);
    }
    let mut ek_bytes = [0u8; EK_LEN];
    ek_bytes.copy_from_slice(ek_slice);

    let dk_raw = dk.key_bytes().map_err(|_| RatchetError::KemKeygen)?;
    let dk_slice = dk_raw.as_ref();
    if dk_slice.len() != DK_LEN {
        return Err(RatchetError::KemKeygen);
    }
    let mut dk_bytes = Zeroizing::new([0u8; DK_LEN]);
    dk_bytes.copy_from_slice(dk_slice);

    Ok(MlKemKeyMaterial { dk_bytes, ek_bytes })
}

/// Encapsulate against an ML-KEM-768 public key.
///
/// Returns `(ciphertext[1088], shared_secret[32])`.
pub fn encaps(ek_bytes: &[u8; EK_LEN]) -> Result<([u8; CT_LEN], Zeroizing<[u8; SS_LEN]>), RatchetError> {
    let ek = aws_lc_rs::kem::EncapsulationKey::new(&ML_KEM_768, ek_bytes)
        .map_err(|_| RatchetError::KemEkInvalid)?;
    let (ct, ss) = ek.encapsulate().map_err(|_| RatchetError::KemEncaps)?;
    if ct.as_ref().len() != CT_LEN || ss.as_ref().len() != SS_LEN {
        return Err(RatchetError::KemEncaps);
    }
    let mut ct_out = [0u8; CT_LEN];
    ct_out.copy_from_slice(ct.as_ref());
    let mut ss_out = Zeroizing::new([0u8; SS_LEN]);
    ss_out.copy_from_slice(ss.as_ref());
    Ok((ct_out, ss_out))
}

/// Decapsulate an ML-KEM-768 ciphertext.
///
/// Returns the 32-byte shared secret.
pub fn decaps(
    dk_bytes: &Zeroizing<[u8; DK_LEN]>,
    ct_bytes: &[u8; CT_LEN],
) -> Result<Zeroizing<[u8; SS_LEN]>, RatchetError> {
    let dk = DecapsulationKey::new(&ML_KEM_768, dk_bytes.as_ref())
        .map_err(|_| RatchetError::KemDecaps)?;
    let ct = aws_lc_rs::kem::Ciphertext::from(ct_bytes.as_slice());
    let ss = dk.decapsulate(ct).map_err(|_| RatchetError::KemDecaps)?;
    if ss.as_ref().len() != SS_LEN {
        return Err(RatchetError::KemDecaps);
    }
    let mut out = Zeroizing::new([0u8; SS_LEN]);
    out.copy_from_slice(ss.as_ref());
    Ok(out)
}
