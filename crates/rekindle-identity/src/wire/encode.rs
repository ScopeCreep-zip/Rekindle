//! RID/1 deterministic CBOR profile encoder.
//!
//! Five primitives — `head()`, `bytes()`, `text()`, `array_head()`,
//! and `unsigned()` — sufficient for every wire object in the identity
//! protocol. No maps, no tags, no floats, no indefinite lengths.
//!
//! Conforms to RFC 8949 §4.2 Core Deterministic Encoding Requirements:
//! - Integers use the smallest possible head encoding (minimal width).
//! - Lengths are definite (no indefinite-length forms).
//! - Map keys are sorted (moot: no maps in this profile).
//!
//! Within the RID type subset (unsigned integers, definite byte strings,
//! definite text strings, definite arrays), this output is byte-identical
//! to any conformant dCBOR encoder restricted to the same subset.

/// CBOR major type constants (upper 3 bits of the initial byte).
const MAJOR_UNSIGNED: u8 = 0; // major 0
const MAJOR_BYTES: u8 = 2;    // major 2
const MAJOR_TEXT: u8 = 3;     // major 3
const MAJOR_ARRAY: u8 = 4;    // major 4

/// Encode the CBOR head (major type + argument) using minimal width.
///
/// RFC 8949 §4.2.1: the argument is encoded in the shortest form that
/// can represent the value:
/// - 0..=23: argument in the initial byte itself (1 byte total)
/// - 24..=255: initial byte + 1 argument byte (2 bytes total)
/// - 256..=65535: initial byte + 2 argument bytes (3 bytes total)
/// - 65536..=4294967295: initial byte + 4 argument bytes (5 bytes total)
/// - Larger: initial byte + 8 argument bytes (9 bytes total)
///
/// This function is the single point where width selection happens.
/// Every encoding function below delegates to it. If this function is
/// correct, every output is minimal-width by construction.
pub fn head(buf: &mut Vec<u8>, major: u8, value: u64) {
    let mt = major << 5;
    match value {
        0..=23 => {
            buf.push(mt | value as u8);
        }
        24..=255 => {
            buf.push(mt | 24);
            buf.push(value as u8);
        }
        256..=65535 => {
            buf.push(mt | 25);
            buf.extend_from_slice(&(value as u16).to_be_bytes());
        }
        65536..=4_294_967_295 => {
            buf.push(mt | 26);
            buf.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            buf.push(mt | 27);
            buf.extend_from_slice(&value.to_be_bytes());
        }
    }
}

/// Encode a CBOR unsigned integer (major type 0).
pub fn unsigned(buf: &mut Vec<u8>, value: u64) {
    head(buf, MAJOR_UNSIGNED, value);
}

/// Encode a CBOR definite-length byte string (major type 2).
pub fn bytes(buf: &mut Vec<u8>, data: &[u8]) {
    head(buf, MAJOR_BYTES, data.len() as u64);
    buf.extend_from_slice(data);
}

/// Encode a CBOR definite-length text string (major type 3).
///
/// The caller MUST ensure `s` is valid UTF-8 (guaranteed by Rust's `&str`).
pub fn text(buf: &mut Vec<u8>, s: &str) {
    head(buf, MAJOR_TEXT, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

/// Encode a CBOR definite-length array header (major type 4).
///
/// The caller MUST subsequently encode exactly `count` items.
/// This function emits only the head; the items follow.
pub fn array_head(buf: &mut Vec<u8>, count: u64) {
    head(buf, MAJOR_ARRAY, count);
}

/// Encode a boolean as a CBOR simple value.
///
/// false = 0xF4 (major 7, value 20)
/// true  = 0xF5 (major 7, value 21)
pub fn boolean(buf: &mut Vec<u8>, value: bool) {
    buf.push(if value { 0xF5 } else { 0xF4 });
}

/// Encode CBOR null (simple value 22).
pub fn null(buf: &mut Vec<u8>) {
    buf.push(0xF6);
}

/// Encode an optional value: null if None, delegate to `encode_fn` if Some.
pub fn optional<F>(buf: &mut Vec<u8>, value: &Option<impl AsRef<[u8]>>, encode_fn: F)
where
    F: FnOnce(&mut Vec<u8>, &[u8]),
{
    match value {
        None => null(buf),
        Some(v) => encode_fn(buf, v.as_ref()),
    }
}

/// Pre-compute the CBOR text string encoding of a `&'static str` at
/// compile time (conceptually — actual computation is on first call,
/// then cached). Used for domain prefix bytes in the Signable trait.
///
/// Returns the head bytes + UTF-8 bytes as a single `Vec<u8>`.
pub fn text_precomputed(s: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + s.len());
    text(&mut buf, s);
    buf
}

/// Encode an HLC timestamp as a 2-element CBOR array:
/// `[physical_ns: uint, logical: uint]`.
pub fn hlc(buf: &mut Vec<u8>, physical_ns: u64, logical: u32) {
    array_head(buf, 2);
    unsigned(buf, physical_ns);
    unsigned(buf, logical as u64);
}

/// Encode a 64-byte Ed25519 signature as a CBOR byte string.
pub fn signature(buf: &mut Vec<u8>, sig: &[u8; 64]) {
    bytes(buf, sig);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_minimal_width_inline() {
        // 0..=23 encode in 1 byte
        for v in 0..=23u64 {
            let mut buf = Vec::new();
            head(&mut buf, MAJOR_UNSIGNED, v);
            assert_eq!(buf.len(), 1, "value {v} should encode in 1 byte");
            assert_eq!(buf[0], v as u8, "value {v} should be inline");
        }
    }

    #[test]
    fn head_minimal_width_one_byte() {
        // 24..=255 encode in 2 bytes
        for v in [24u64, 100, 255] {
            let mut buf = Vec::new();
            head(&mut buf, MAJOR_UNSIGNED, v);
            assert_eq!(buf.len(), 2, "value {v} should encode in 2 bytes");
            assert_eq!(buf[0], 24, "value {v} should use additional info 24");
            assert_eq!(buf[1], v as u8);
        }
    }

    #[test]
    fn head_minimal_width_two_byte() {
        // 256..=65535 encode in 3 bytes
        for v in [256u64, 1000, 65535] {
            let mut buf = Vec::new();
            head(&mut buf, MAJOR_UNSIGNED, v);
            assert_eq!(buf.len(), 3, "value {v} should encode in 3 bytes");
            assert_eq!(buf[0], 25, "value {v} should use additional info 25");
            let decoded = u16::from_be_bytes([buf[1], buf[2]]);
            assert_eq!(decoded as u64, v);
        }
    }

    #[test]
    fn head_minimal_width_four_byte() {
        // 65536..=4294967295 encode in 5 bytes
        for v in [65536u64, 1_000_000, 4_294_967_295] {
            let mut buf = Vec::new();
            head(&mut buf, MAJOR_UNSIGNED, v);
            assert_eq!(buf.len(), 5, "value {v} should encode in 5 bytes");
            assert_eq!(buf[0], 26);
            let decoded = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
            assert_eq!(decoded as u64, v);
        }
    }

    #[test]
    fn head_minimal_width_eight_byte() {
        let v = 4_294_967_296u64;
        let mut buf = Vec::new();
        head(&mut buf, MAJOR_UNSIGNED, v);
        assert_eq!(buf.len(), 9, "value {v} should encode in 9 bytes");
        assert_eq!(buf[0], 27);
        let decoded = u64::from_be_bytes([buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8]]);
        assert_eq!(decoded, v);
    }

    #[test]
    fn bytes_encoding() {
        let mut buf = Vec::new();
        bytes(&mut buf, &[0xDE, 0xAD]);
        // major 2, length 2 (inline), then the two bytes
        assert_eq!(buf, vec![0x42, 0xDE, 0xAD]);
    }

    #[test]
    fn bytes_empty() {
        let mut buf = Vec::new();
        bytes(&mut buf, &[]);
        assert_eq!(buf, vec![0x40]); // major 2, length 0
    }

    #[test]
    fn bytes_32_byte_key() {
        let key = [0xAA; 32];
        let mut buf = Vec::new();
        bytes(&mut buf, &key);
        // length 32 > 23, so 2-byte head: 0x58 (major 2, additional 24), 0x20 (32)
        assert_eq!(buf[0], 0x58);
        assert_eq!(buf[1], 32);
        assert_eq!(&buf[2..], &key[..]);
        assert_eq!(buf.len(), 34);
    }

    #[test]
    fn text_encoding() {
        let mut buf = Vec::new();
        text(&mut buf, "hello");
        // major 3, length 5 (inline), then UTF-8 bytes
        assert_eq!(buf, vec![0x65, b'h', b'e', b'l', b'l', b'o']);
    }

    #[test]
    fn text_empty() {
        let mut buf = Vec::new();
        text(&mut buf, "");
        assert_eq!(buf, vec![0x60]); // major 3, length 0
    }

    #[test]
    fn array_head_encoding() {
        let mut buf = Vec::new();
        array_head(&mut buf, 3);
        assert_eq!(buf, vec![0x83]); // major 4, length 3 inline
    }

    #[test]
    fn boolean_encoding() {
        let mut buf = Vec::new();
        boolean(&mut buf, false);
        boolean(&mut buf, true);
        assert_eq!(buf, vec![0xF4, 0xF5]);
    }

    #[test]
    fn null_encoding() {
        let mut buf = Vec::new();
        null(&mut buf);
        assert_eq!(buf, vec![0xF6]);
    }

    #[test]
    fn hlc_encoding() {
        let mut buf = Vec::new();
        hlc(&mut buf, 1_000_000_000, 42);
        // 2-element array, then two unsigned integers
        assert_eq!(buf[0], 0x82); // array(2)
        // 1_000_000_000 fits in 4 bytes
        assert_eq!(buf[1], 0x1A); // uint32 head
        let phys = u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]);
        assert_eq!(phys, 1_000_000_000);
        // 42 is in 24..=255 range: one-byte head (0x18) + value byte (42)
        assert_eq!(buf[6], 0x18);
        assert_eq!(buf[7], 42);
    }

    #[test]
    fn signature_encoding() {
        let sig = [0xBB; 64];
        let mut buf = Vec::new();
        signature(&mut buf, &sig);
        // 64 bytes: head = 0x58 (byte string, 1-byte length), 0x40 (64)
        assert_eq!(buf[0], 0x58);
        assert_eq!(buf[1], 64);
        assert_eq!(&buf[2..], &sig[..]);
    }

    #[test]
    fn deterministic_across_calls() {
        // Same inputs MUST produce same bytes every time.
        for _ in 0..100 {
            let mut a = Vec::new();
            let mut b = Vec::new();
            unsigned(&mut a, 12345);
            bytes(&mut a, &[1, 2, 3]);
            text(&mut a, "test");
            array_head(&mut a, 2);

            unsigned(&mut b, 12345);
            bytes(&mut b, &[1, 2, 3]);
            text(&mut b, "test");
            array_head(&mut b, 2);

            assert_eq!(a, b);
        }
    }

    #[test]
    fn major_type_encoding() {
        // Verify major type bits are correctly shifted for each type
        let mut buf = Vec::new();

        // Unsigned (major 0): high bits = 000
        unsigned(&mut buf, 5);
        assert_eq!(buf[0] >> 5, 0);
        buf.clear();

        // Bytes (major 2): high bits = 010
        bytes(&mut buf, &[0]);
        assert_eq!(buf[0] >> 5, 2);
        buf.clear();

        // Text (major 3): high bits = 011
        text(&mut buf, "x");
        assert_eq!(buf[0] >> 5, 3);
        buf.clear();

        // Array (major 4): high bits = 100
        array_head(&mut buf, 1);
        assert_eq!(buf[0] >> 5, 4);
    }
}
