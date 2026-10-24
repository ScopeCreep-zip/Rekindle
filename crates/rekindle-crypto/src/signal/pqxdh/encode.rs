//! `EncodeEC` and `EncodeKEM` (PQXDH rev. 3 §2.3): a public key as the
//! KDF input, the AD and (from plan E2.4) the prekey signatures bind it —
//! one type byte naming the algorithm, then the key's own encoding.

use ed25519_dalek::VerifyingKey;
use rekindle_secrets::pq_keys::MlKemPublic;
use x25519_dalek::PublicKey as X25519Public;

/// Type byte of a Curve25519 key (`u`-coordinate): the byte Signal's
/// deployed clients use (PQXDH rev. 3 §2.3 leaves the constant to the
/// application).
pub const X25519_TYPE: u8 = 0x05;

/// Type byte of an Ed25519 identity key (RFC 8032 point encoding).
/// Rekindle's own: Signal's identities are Curve25519 keys signed with
/// XEdDSA, ours are Ed25519 keys and the address is that key, whose
/// Montgomery `u` drops the sign bit.
pub const ED25519_TYPE: u8 = 0x06;

/// Type byte of an ML-KEM-768 key. Rekindle's own, apart from the bytes
/// Signal's deployed clients give other KEMs (0x07, 0x08, 0x0A).
pub const ML_KEM_768_TYPE: u8 = 0x09;

/// Length of an encoded curve key: the type byte and 32 key bytes.
pub const ENCODED_EC_LEN: usize = 33;

/// `EncodeEC` of a Curve25519 key: the type byte, then `u` little-endian.
#[must_use]
pub fn encode_ec(key: &X25519Public) -> [u8; ENCODED_EC_LEN] {
    typed(X25519_TYPE, key.as_bytes())
}

/// `EncodeEC` of an identity key. The Ed25519 encoding, not its
/// Montgomery form, so the bytes bound are exactly the address.
#[must_use]
pub fn encode_identity(key: &VerifyingKey) -> [u8; ENCODED_EC_LEN] {
    typed(ED25519_TYPE, key.as_bytes())
}

/// `EncodeKEM` of an ML-KEM-768 key: the type byte, then FIPS 203's
/// 1184-byte encapsulation key.
#[must_use]
pub fn encode_kem(key: &MlKemPublic) -> Vec<u8> {
    let bytes = key.as_bytes();
    let mut out = Vec::with_capacity(1 + bytes.len());
    out.push(ML_KEM_768_TYPE);
    out.extend_from_slice(bytes);
    out
}

fn typed(kind: u8, key: &[u8; 32]) -> [u8; ENCODED_EC_LEN] {
    let mut out = [0u8; ENCODED_EC_LEN];
    out[0] = kind;
    out[1..].copy_from_slice(key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_secrets::pq_keys::MlKemSecret;

    #[test]
    fn the_type_byte_leads_and_the_key_follows() {
        let x = X25519Public::from([9u8; 32]);
        assert_eq!(encode_ec(&x)[0], X25519_TYPE);
        assert_eq!(&encode_ec(&x)[1..], x.as_bytes());

        let identity = crate::Identity::generate();
        let encoded = encode_identity(&identity.public_key());
        assert_eq!(encoded[0], ED25519_TYPE);
        assert_eq!(encoded[1..], identity.public_key_bytes());

        let (_, kem) = MlKemSecret::generate();
        let encoded = encode_kem(&kem);
        assert_eq!(encoded[0], ML_KEM_768_TYPE);
        assert_eq!(&encoded[1..], kem.as_bytes());
    }

    #[test]
    fn an_identity_and_its_montgomery_form_encode_apart() {
        let identity = crate::Identity::generate();
        assert_ne!(
            encode_identity(&identity.public_key()),
            encode_ec(&identity.to_x25519_public())
        );
    }
}
