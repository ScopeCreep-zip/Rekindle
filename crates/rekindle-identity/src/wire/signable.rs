//! Signable trait and CBOR Sequence framing for wire objects.
//!
//! Every signed wire object in the identity protocol implements `Signable`.
//! The signable form is a **CBOR Sequence** (RFC 8742): the domain prefix
//! text string concatenated with the field array. This is NOT a single
//! CBOR item — it is two items concatenated. The domain prefix is a
//! compile-time constant; the field array contains all fields EXCEPT
//! the signature itself.
//!
//! Framing: `signable_bytes = TEXT(SIGN_DOMAIN) ‖ ARRAY(fields…)`
//!
//! The domain prefix bytes are fully const-evaluable (the TEXT head +
//! UTF-8 of a `&'static str`), so the hot path's signable reconstruction
//! is `buf.extend(DOMAIN_PREFIX_BYTES)` + field encoding — no per-call
//! domain encoding.

use super::encode;

/// A wire object that can be signed and verified.
///
/// Implementors MUST:
/// - Return a `SIGN_DOMAIN` that is one of the constants from
///   `crate::origin::tags::derivation_tags`.
/// - Encode all fields EXCEPT signature(s) into `signable_bytes()`.
/// - Use `encode_signable()` to produce the CBOR Sequence framing.
///
/// The field array uses positional encoding (definite-length CBOR array,
/// fields in declaration order). Field order is the wire format — any
/// reorder is a wire-breaking change caught by pinned test vectors.
pub trait Signable {
    /// The domain separation string. Must be a `&'static str` from
    /// `derivation_tags`. Used as the CBOR text string prefix in the
    /// signable sequence.
    const SIGN_DOMAIN: &'static str;

    /// Encode the signable form: domain prefix + field array.
    ///
    /// The default implementation calls `encode_fields()` to get the
    /// field array bytes, then prepends the domain prefix.
    fn signable_bytes(&self) -> Vec<u8> {
        encode_signable(Self::SIGN_DOMAIN, |buf| self.encode_fields(buf))
    }

    /// Encode just the fields (the CBOR array contents including the
    /// array head). Called by the default `signable_bytes()`.
    ///
    /// Implementors write the complete array: `array_head(n)` followed
    /// by `n` field encodings.
    fn encode_fields(&self, buf: &mut Vec<u8>);
}

/// Produce the signable CBOR Sequence: `TEXT(domain) ‖ ARRAY(fields)`.
///
/// `encode_fn` is called with a buffer that already contains the domain
/// prefix. It MUST write a single CBOR array (head + elements).
pub fn encode_signable<F>(domain: &str, encode_fn: F) -> Vec<u8>
where
    F: FnOnce(&mut Vec<u8>),
{
    // Pre-compute capacity estimate: domain prefix + typical field array.
    // Domain strings are ~30 bytes; field arrays are ~200 bytes.
    let mut buf = Vec::with_capacity(256);

    // Domain prefix as CBOR text string.
    encode::text(&mut buf, domain);

    // Field array — the implementor writes the array head + elements.
    encode_fn(&mut buf);

    buf
}

/// Compute the pre-encoded domain prefix bytes for a static domain string.
///
/// Returns the CBOR TEXT encoding of the domain. Used by implementations
/// that want to cache the prefix for repeated signing (e.g., batch
/// operations). The returned bytes are the exact prefix of signable_bytes().
pub fn domain_prefix_bytes(domain: &str) -> Vec<u8> {
    encode::text_precomputed(domain)
}

/// A 64-byte Ed25519 signature.
///
/// Transparent wrapper for clarity in wire object struct fields.
/// Encodes as a CBOR byte string (major 2, length 64).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature64(pub [u8; 64]);

impl Signature64 {
    pub const ZERO: Self = Self([0u8; 64]);

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }

    pub fn from_bytes(b: [u8; 64]) -> Self {
        Self(b)
    }

    /// Encode into a CBOR buffer.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        encode::signature(buf, &self.0);
    }
}

impl core::fmt::Debug for Signature64 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = hex::encode(&self.0[..4]);
        write!(f, "Sig64({h}…)")
    }
}

impl serde::Serialize for Signature64 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> serde::Deserialize<'de> for Signature64 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "signature must be 64 bytes, got {}", bytes.len()
            )));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

/// A 96-bit hybrid logical clock timestamp.
///
/// `physical_ns`: nanoseconds since Unix epoch.
/// `logical`: monotonic counter within the same physical timestamp.
///
/// Encodes as a 2-element CBOR array: `[physical_ns, logical]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Hlc {
    pub physical_ns: u64,
    pub logical: u32,
}

impl Hlc {
    pub fn new(physical_ns: u64, logical: u32) -> Self {
        Self { physical_ns, logical }
    }

    /// Current wall-clock time with logical 0. For use in non-HLC-tracked
    /// contexts (origination, manual operations). Production code should
    /// use a proper HLC implementation that maintains the logical counter.
    pub fn now() -> Self {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self { physical_ns: ns, logical: 0 }
    }

    /// Encode into a CBOR buffer.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        encode::hlc(buf, self.physical_ns, self.logical);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::decode;

    /// A minimal Signable implementor for testing.
    struct TestObject {
        value_a: u64,
        value_b: [u8; 32],
    }

    impl Signable for TestObject {
        const SIGN_DOMAIN: &'static str = "test domain v1";

        fn encode_fields(&self, buf: &mut Vec<u8>) {
            encode::array_head(buf, 2);
            encode::unsigned(buf, self.value_a);
            encode::bytes(buf, &self.value_b);
        }
    }

    #[test]
    fn signable_starts_with_domain_text() {
        let obj = TestObject { value_a: 42, value_b: [0xAA; 32] };
        let bytes = obj.signable_bytes();

        // Decode as a sequence: first item should be the domain text
        let (domain, fields, consumed) = decode::decode_sequence_head(&bytes).unwrap();
        assert_eq!(consumed, bytes.len(), "sequence must consume all bytes when no trailing data");
        assert_eq!(domain, "test domain v1");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].as_unsigned().unwrap(), 42);
        assert_eq!(fields[1].as_bytes().unwrap(), &[0xAA; 32]);
    }

    #[test]
    fn signable_deterministic() {
        let obj = TestObject { value_a: 99, value_b: [0xBB; 32] };
        let a = obj.signable_bytes();
        let b = obj.signable_bytes();
        assert_eq!(a, b);
    }

    #[test]
    fn signable_different_values_different_bytes() {
        let obj1 = TestObject { value_a: 1, value_b: [0x00; 32] };
        let obj2 = TestObject { value_a: 2, value_b: [0x00; 32] };
        assert_ne!(obj1.signable_bytes(), obj2.signable_bytes());
    }

    #[test]
    fn domain_prefix_cached() {
        let prefix = domain_prefix_bytes("rekindle identity rotation v1");
        let mut expected = Vec::new();
        encode::text(&mut expected, "rekindle identity rotation v1");
        assert_eq!(prefix, expected);
    }

    #[test]
    fn signature64_roundtrip_serde() {
        let sig = Signature64([0xCC; 64]);
        let json = serde_json::to_string(&sig).unwrap();
        let restored: Signature64 = serde_json::from_str(&json).unwrap();
        assert_eq!(sig, restored);
    }

    #[test]
    fn hlc_encode_decode() {
        let ts = Hlc::new(1_700_000_000_000_000_000, 42);
        let mut buf = Vec::new();
        ts.encode_into(&mut buf);
        let item = decode::decode(&buf).unwrap();
        let arr = item.as_array().unwrap();
        assert_eq!(arr[0].as_unsigned().unwrap(), 1_700_000_000_000_000_000);
        assert_eq!(arr[1].as_unsigned().unwrap(), 42);
    }

    #[test]
    fn hlc_ordering() {
        let a = Hlc::new(100, 0);
        let b = Hlc::new(100, 1);
        let c = Hlc::new(101, 0);
        assert!(a < b);
        assert!(b < c);
    }
}
