//! HKDF key derivation — 9 keys from one handshake hash.

use aws_lc_rs::hkdf::{Salt, HKDF_SHA256, KeyType};

use crate::v3::wire::constants::{
    LABEL_ENVELOPE_D2L, LABEL_ENVELOPE_L2D,
    LABEL_HEADER_D2L, LABEL_HEADER_L2D,
    LABEL_STREAM_D2L, LABEL_STREAM_L2D,
    LABEL_AUDIT_D2L, LABEL_AUDIT_L2D,
    LABEL_HANDOFF,
};

/// All symmetric keys derived from a single handshake hash.
#[derive(Debug, Clone)]
pub struct DerivedKeys {
    pub envelope_d2l: [u8; 32],
    pub envelope_l2d: [u8; 32],
    pub header_d2l: [u8; 32],
    pub header_l2d: [u8; 32],
    pub stream_d2l: [u8; 32],
    pub stream_l2d: [u8; 32],
    pub audit_d2l: [u8; 32],
    pub audit_l2d: [u8; 32],
    pub handoff: [u8; 32],
}

/// Length specifier for HKDF expand — tells aws-lc-rs to produce 32 bytes.
struct HkdfLen;

impl KeyType for HkdfLen {
    fn len(&self) -> usize { 32 }
}

fn hkdf_expand(prk: &aws_lc_rs::hkdf::Prk, label: &str) -> [u8; 32] {
    let info = [label.as_bytes()];
    let okm = prk.expand(&info, HkdfLen)
        .expect("HKDF expand failed");
    let mut out = [0u8; 32];
    okm.fill(&mut out).expect("HKDF fill failed");
    out
}

/// Derive all 9 keys from the Noise handshake hash via HKDF-SHA256.
pub fn derive_all_keys(handshake_hash: &[u8; 32]) -> DerivedKeys {
    derive_from_ikm(handshake_hash)
}

/// Derive all 9 keys from a rotation combined_secret via HKDF-SHA256.
/// Uses the same domain-separated labels as the handshake derivation.
pub fn derive_rotation_keys(combined_secret: &[u8; 32]) -> DerivedKeys {
    derive_from_ikm(combined_secret)
}

fn derive_from_ikm(ikm: &[u8; 32]) -> DerivedKeys {
    let salt = Salt::new(HKDF_SHA256, &[]);
    let prk = salt.extract(ikm);

    DerivedKeys {
        envelope_d2l: hkdf_expand(&prk, LABEL_ENVELOPE_D2L),
        envelope_l2d: hkdf_expand(&prk, LABEL_ENVELOPE_L2D),
        header_d2l: hkdf_expand(&prk, LABEL_HEADER_D2L),
        header_l2d: hkdf_expand(&prk, LABEL_HEADER_L2D),
        stream_d2l: hkdf_expand(&prk, LABEL_STREAM_D2L),
        stream_l2d: hkdf_expand(&prk, LABEL_STREAM_L2D),
        audit_d2l: hkdf_expand(&prk, LABEL_AUDIT_D2L),
        audit_l2d: hkdf_expand(&prk, LABEL_AUDIT_L2D),
        handoff: hkdf_expand(&prk, LABEL_HANDOFF),
    }
}
