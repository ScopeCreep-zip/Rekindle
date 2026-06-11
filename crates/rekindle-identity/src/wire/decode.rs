//! RID/1 deterministic CBOR profile decoder.
//!
//! Hardened parser enforcing the RID/1 profile restrictions:
//! - No indefinite-length forms (rejected)
//! - No tags (major 6 rejected)
//! - No maps (major 5 rejected)
//! - No floats (major 7 additional 25/26/27 rejected)
//! - No simple values other than true (21), false (20), null (22)
//! - Maximum nesting depth: 4
//! - Maximum array element count: 16
//! - Trailing bytes after the top-level object: rejected
//! - Non-minimal integer encoding: rejected
//!
//! The decoder operates on a cursor over `&[u8]` and returns structured
//! items. It never allocates except when returning owned byte vectors
//! for byte string payloads.

use crate::error::IdentityError;

/// Maximum nesting depth for arrays.
const MAX_NESTING: usize = 4;

/// Maximum number of elements in a single array.
const MAX_ELEMENTS: u64 = 16;

/// A decoded CBOR item in the RID/1 profile.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Unsigned integer (major 0).
    Unsigned(u64),
    /// Byte string (major 2), owned.
    Bytes(Vec<u8>),
    /// Text string (major 3), owned.
    Text(String),
    /// Array of items (major 4).
    Array(Vec<Item>),
    /// Boolean (simple values 20/21).
    Bool(bool),
    /// Null (simple value 22).
    Null,
}

impl Item {
    /// Extract as unsigned integer or error.
    pub fn as_unsigned(&self) -> Result<u64, IdentityError> {
        match self {
            Item::Unsigned(v) => Ok(*v),
            other => Err(IdentityError::Encoding(format!("expected unsigned, got {}", other.type_name()))),
        }
    }

    /// Extract as byte slice or error.
    pub fn as_bytes(&self) -> Result<&[u8], IdentityError> {
        match self {
            Item::Bytes(v) => Ok(v),
            other => Err(IdentityError::Encoding(format!("expected bytes, got {}", other.type_name()))),
        }
    }

    /// Extract as string slice or error.
    pub fn as_text(&self) -> Result<&str, IdentityError> {
        match self {
            Item::Text(v) => Ok(v),
            other => Err(IdentityError::Encoding(format!("expected text, got {}", other.type_name()))),
        }
    }

    /// Extract as array slice or error.
    pub fn as_array(&self) -> Result<&[Item], IdentityError> {
        match self {
            Item::Array(v) => Ok(v),
            other => Err(IdentityError::Encoding(format!("expected array, got {}", other.type_name()))),
        }
    }

    /// Extract as bool or error.
    pub fn as_bool(&self) -> Result<bool, IdentityError> {
        match self {
            Item::Bool(v) => Ok(*v),
            other => Err(IdentityError::Encoding(format!("expected bool, got {}", other.type_name()))),
        }
    }

    /// Extract as fixed-size byte array or error.
    pub fn as_bytes_fixed<const N: usize>(&self) -> Result<[u8; N], IdentityError> {
        let b = self.as_bytes()?;
        if b.len() != N {
            return Err(IdentityError::Encoding(format!(
                "expected {N} bytes, got {}", b.len()
            )));
        }
        let mut arr = [0u8; N];
        arr.copy_from_slice(b);
        Ok(arr)
    }

    fn type_name(&self) -> &'static str {
        match self {
            Item::Unsigned(_) => "unsigned",
            Item::Bytes(_) => "bytes",
            Item::Text(_) => "text",
            Item::Array(_) => "array",
            Item::Bool(_) => "bool",
            Item::Null => "null",
        }
    }
}

/// Decode cursor — tracks position and nesting depth.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, depth: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn peek(&self) -> Result<u8, IdentityError> {
        if self.pos >= self.data.len() {
            return Err(IdentityError::Encoding("unexpected end of input".into()));
        }
        Ok(self.data[self.pos])
    }

    fn read_byte(&mut self) -> Result<u8, IdentityError> {
        if self.pos >= self.data.len() {
            return Err(IdentityError::Encoding("unexpected end of input".into()));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_exact(&mut self, n: usize) -> Result<&'a [u8], IdentityError> {
        if self.pos + n > self.data.len() {
            return Err(IdentityError::Encoding(format!(
                "need {n} bytes at offset {}, only {} available",
                self.pos, self.remaining()
            )));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Read the argument value from a CBOR head, enforcing minimal width.
    fn read_argument(&mut self, additional: u8) -> Result<u64, IdentityError> {
        match additional {
            0..=23 => Ok(additional as u64),
            24 => {
                let v = self.read_byte()? as u64;
                if v < 24 {
                    return Err(IdentityError::Encoding(format!(
                        "non-minimal encoding: value {v} in 1-byte form (should be inline)"
                    )));
                }
                Ok(v)
            }
            25 => {
                let b = self.read_exact(2)?;
                let v = u16::from_be_bytes([b[0], b[1]]) as u64;
                if v <= 255 {
                    return Err(IdentityError::Encoding(format!(
                        "non-minimal encoding: value {v} in 2-byte form"
                    )));
                }
                Ok(v)
            }
            26 => {
                let b = self.read_exact(4)?;
                let v = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64;
                if v <= 65535 {
                    return Err(IdentityError::Encoding(format!(
                        "non-minimal encoding: value {v} in 4-byte form"
                    )));
                }
                Ok(v)
            }
            27 => {
                let b = self.read_exact(8)?;
                let v = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                if v <= 4_294_967_295 {
                    return Err(IdentityError::Encoding(format!(
                        "non-minimal encoding: value {v} in 8-byte form"
                    )));
                }
                Ok(v)
            }
            28..=30 => Err(IdentityError::Encoding(format!(
                "reserved additional info value {additional}"
            ))),
            31 => Err(IdentityError::Encoding(
                "indefinite length (additional info 31) is prohibited in RID/1".into()
            )),
            _ => unreachable!("additional is masked to 5 bits"),
        }
    }

    /// Decode one CBOR item, enforcing RID/1 profile restrictions.
    fn decode_item(&mut self) -> Result<Item, IdentityError> {
        let initial = self.read_byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1F;

        match major {
            // Major 0: unsigned integer
            0 => {
                let v = self.read_argument(additional)?;
                Ok(Item::Unsigned(v))
            }

            // Major 1: negative integer — not used in RID/1
            1 => Err(IdentityError::Encoding(
                "negative integers (major 1) are not used in RID/1".into()
            )),

            // Major 2: byte string
            2 => {
                if additional == 31 {
                    return Err(IdentityError::Encoding(
                        "indefinite-length byte string prohibited".into()
                    ));
                }
                let len = self.read_argument(additional)?;
                // Sanity limit: no single byte string > 16 MiB
                if len > 16 * 1024 * 1024 {
                    return Err(IdentityError::Encoding(format!(
                        "byte string length {len} exceeds sanity limit"
                    )));
                }
                let data = self.read_exact(len as usize)?;
                Ok(Item::Bytes(data.to_vec()))
            }

            // Major 3: text string
            3 => {
                if additional == 31 {
                    return Err(IdentityError::Encoding(
                        "indefinite-length text string prohibited".into()
                    ));
                }
                let len = self.read_argument(additional)?;
                if len > 16 * 1024 * 1024 {
                    return Err(IdentityError::Encoding(format!(
                        "text string length {len} exceeds sanity limit"
                    )));
                }
                let data = self.read_exact(len as usize)?;
                let s = core::str::from_utf8(data).map_err(|e| {
                    IdentityError::Encoding(format!("invalid UTF-8 in text string: {e}"))
                })?;
                Ok(Item::Text(s.to_owned()))
            }

            // Major 4: array
            4 => {
                if additional == 31 {
                    return Err(IdentityError::Encoding(
                        "indefinite-length array prohibited".into()
                    ));
                }
                if self.depth >= MAX_NESTING {
                    return Err(IdentityError::Encoding(format!(
                        "array nesting depth {} exceeds maximum {MAX_NESTING}",
                        self.depth + 1
                    )));
                }
                let count = self.read_argument(additional)?;
                if count > MAX_ELEMENTS {
                    return Err(IdentityError::Encoding(format!(
                        "array element count {count} exceeds maximum {MAX_ELEMENTS}"
                    )));
                }
                self.depth += 1;
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    items.push(self.decode_item()?);
                }
                self.depth -= 1;
                Ok(Item::Array(items))
            }

            // Major 5: map — PROHIBITED
            5 => Err(IdentityError::Encoding(
                "maps (major 5) are prohibited in RID/1".into()
            )),

            // Major 6: tag — PROHIBITED
            6 => Err(IdentityError::Encoding(
                "tags (major 6) are prohibited in RID/1".into()
            )),

            // Major 7: simple values and floats
            7 => {
                match additional {
                    20 => Ok(Item::Bool(false)),
                    21 => Ok(Item::Bool(true)),
                    22 => Ok(Item::Null),
                    // 23 = undefined — reject
                    23 => Err(IdentityError::Encoding(
                        "undefined (simple value 23) is prohibited".into()
                    )),
                    // 24 = simple value in next byte — reject all
                    24 => {
                        let sv = self.read_byte()?;
                        Err(IdentityError::Encoding(format!(
                            "simple value {sv} is not permitted in RID/1"
                        )))
                    }
                    // 25 = half-precision float — PROHIBITED
                    25 => Err(IdentityError::Encoding(
                        "half-precision float (f16) prohibited in RID/1".into()
                    )),
                    // 26 = single-precision float — PROHIBITED
                    26 => Err(IdentityError::Encoding(
                        "single-precision float (f32) prohibited in RID/1".into()
                    )),
                    // 27 = double-precision float — PROHIBITED
                    27 => Err(IdentityError::Encoding(
                        "double-precision float (f64) prohibited in RID/1".into()
                    )),
                    // 31 = break code (only valid inside indefinite)
                    31 => Err(IdentityError::Encoding(
                        "break code (0xFF) outside indefinite-length context".into()
                    )),
                    _ => Err(IdentityError::Encoding(format!(
                        "unassigned simple value (additional {additional}) in major 7"
                    ))),
                }
            }

            _ => unreachable!("major type is 3 bits, 0..=7"),
        }
    }
}

/// Decode a complete RID/1 wire object from bytes.
///
/// Rejects trailing bytes: after decoding one top-level item, any
/// remaining bytes produce an error. This prevents suffix-based
/// confusion attacks where extra data after the signed object is
/// silently ignored.
pub fn decode(data: &[u8]) -> Result<Item, IdentityError> {
    if data.is_empty() {
        return Err(IdentityError::Encoding("empty input".into()));
    }
    let mut cursor = Cursor::new(data);
    let item = cursor.decode_item()?;
    if cursor.pos < data.len() {
        return Err(IdentityError::Encoding(format!(
            "{} trailing bytes after object",
            data.len() - cursor.pos
        )));
    }
    Ok(item)
}

/// Decode a CBOR Sequence (RFC 8742): domain prefix text string followed
/// by a field array. Used for signable form verification.
///
/// Returns `(domain, fields, bytes_consumed)` where `domain` is the text
/// string, `fields` is the array item, and `bytes_consumed` is how many
/// bytes of `data` the sequence occupied. The caller can use this to
/// determine where the signable portion ends (e.g., to find a trailing
/// signature).
pub fn decode_sequence_head(data: &[u8]) -> Result<(String, Vec<Item>, usize), IdentityError> {
    if data.is_empty() {
        return Err(IdentityError::Encoding("empty signable input".into()));
    }
    let mut cursor = Cursor::new(data);

    // First item: domain prefix (text string)
    let domain_item = cursor.decode_item()?;
    let domain = match domain_item {
        Item::Text(s) => s,
        other => return Err(IdentityError::Encoding(format!(
            "signable sequence: expected text domain prefix, got {}",
            other.type_name()
        ))),
    };

    // Second item: field array
    let fields_item = cursor.decode_item()?;
    let fields = match fields_item {
        Item::Array(items) => items,
        other => return Err(IdentityError::Encoding(format!(
            "signable sequence: expected field array, got {}",
            other.type_name()
        ))),
    };

    let consumed = cursor.pos;
    Ok((domain, fields, consumed))
}

/// Decode a complete wire object: signable sequence + trailing signature.
///
/// Parses the domain prefix + field array, then one more byte-string item
/// (the 64-byte signature), then asserts no remaining bytes. Uses `peek`
/// internally to validate frame boundaries before consuming.
///
/// Returns `(domain, fields, signature_bytes)`.
pub fn decode_signed_object(data: &[u8]) -> Result<(String, Vec<Item>, [u8; 64]), IdentityError> {
    if data.is_empty() {
        return Err(IdentityError::Encoding("empty signed object".into()));
    }
    let mut cursor = Cursor::new(data);

    // Domain prefix
    let domain_item = cursor.decode_item()?;
    let domain = match domain_item {
        Item::Text(s) => s,
        other => return Err(IdentityError::Encoding(format!(
            "signed object: expected text domain, got {}", other.type_name()
        ))),
    };

    // Field array
    let fields_item = cursor.decode_item()?;
    let fields = match fields_item {
        Item::Array(items) => items,
        other => return Err(IdentityError::Encoding(format!(
            "signed object: expected field array, got {}", other.type_name()
        ))),
    };

    // Signature (64-byte byte string)
    // Peek to verify there's data remaining before consuming
    let _next = cursor.peek().map_err(|_| {
        IdentityError::Encoding("signed object: missing signature after field array".into())
    })?;
    let sig_item = cursor.decode_item()?;
    let sig_bytes = sig_item.as_bytes_fixed::<64>().map_err(|_| {
        IdentityError::Encoding("signed object: signature must be exactly 64 bytes".into())
    })?;

    // Reject trailing bytes
    if cursor.remaining() > 0 {
        return Err(IdentityError::Encoding(format!(
            "signed object: {} trailing bytes after signature",
            cursor.remaining()
        )));
    }

    Ok((domain, fields, sig_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::encode;

    #[test]
    fn roundtrip_unsigned() {
        for v in [0u64, 1, 23, 24, 255, 256, 65535, 65536, 4_294_967_295, 4_294_967_296, u64::MAX] {
            let mut buf = Vec::new();
            encode::unsigned(&mut buf, v);
            let item = decode(&buf).unwrap();
            assert_eq!(item.as_unsigned().unwrap(), v, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn roundtrip_bytes() {
        for data in [&[][..], &[0xAA], &[0; 32], &[0xFF; 64], &[0; 256]] {
            let mut buf = Vec::new();
            encode::bytes(&mut buf, data);
            let item = decode(&buf).unwrap();
            assert_eq!(item.as_bytes().unwrap(), data);
        }
    }

    #[test]
    fn roundtrip_text() {
        for s in ["", "hello", "rekindle identity rotation v1", &"x".repeat(200)] {
            let mut buf = Vec::new();
            encode::text(&mut buf, s);
            let item = decode(&buf).unwrap();
            assert_eq!(item.as_text().unwrap(), s);
        }
    }

    #[test]
    fn roundtrip_array() {
        let mut buf = Vec::new();
        encode::array_head(&mut buf, 3);
        encode::unsigned(&mut buf, 1);
        encode::unsigned(&mut buf, 2);
        encode::bytes(&mut buf, &[3]);
        let item = decode(&buf).unwrap();
        let arr = item.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_unsigned().unwrap(), 1);
        assert_eq!(arr[1].as_unsigned().unwrap(), 2);
        assert_eq!(arr[2].as_bytes().unwrap(), &[3]);
    }

    #[test]
    fn roundtrip_boolean() {
        let mut buf = Vec::new();
        encode::boolean(&mut buf, false);
        let item = decode(&buf).unwrap();
        assert_eq!(item.as_bool().unwrap(), false);

        buf.clear();
        encode::boolean(&mut buf, true);
        let item = decode(&buf).unwrap();
        assert_eq!(item.as_bool().unwrap(), true);
    }

    #[test]
    fn roundtrip_null() {
        let mut buf = Vec::new();
        encode::null(&mut buf);
        let item = decode(&buf).unwrap();
        assert_eq!(item, Item::Null);
    }

    #[test]
    fn roundtrip_hlc() {
        let mut buf = Vec::new();
        encode::hlc(&mut buf, 1_700_000_000_000_000_000, 7);
        let item = decode(&buf).unwrap();
        let arr = item.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_unsigned().unwrap(), 1_700_000_000_000_000_000);
        assert_eq!(arr[1].as_unsigned().unwrap(), 7);
    }

    // ── Rejection tests ────────────────────────────────────────

    #[test]
    fn rejects_empty_input() {
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut buf = Vec::new();
        encode::unsigned(&mut buf, 42);
        buf.push(0xFF); // trailing garbage
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_trailing_kilobyte() {
        let mut buf = Vec::new();
        encode::unsigned(&mut buf, 42);
        buf.extend_from_slice(&[0; 1024]);
        let err = decode(&buf).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("trailing"), "error should mention trailing: {msg}");
    }

    #[test]
    fn rejects_overlong_unsigned_24_for_23() {
        // Value 23 encoded as 1-byte form (should be inline)
        let buf = vec![0x18, 23]; // major 0, additional 24, value 23
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_overlong_unsigned_24_for_0() {
        let buf = vec![0x18, 0]; // value 0 in 1-byte form
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_overlong_unsigned_25_for_255() {
        // Value 255 in 2-byte form (should be 1-byte)
        let buf = vec![0x19, 0x00, 0xFF];
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_overlong_unsigned_26_for_65535() {
        // Value 65535 in 4-byte form (should be 2-byte)
        let buf = vec![0x1A, 0x00, 0x00, 0xFF, 0xFF];
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_overlong_unsigned_27_for_max_u32() {
        // Value 4294967295 in 8-byte form (should be 4-byte)
        let buf = vec![0x1B, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_indefinite_byte_string() {
        let buf = vec![0x5F, 0x41, 0xAA, 0xFF]; // indefinite bytes, one chunk, break
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_indefinite_text_string() {
        let buf = vec![0x7F, 0x61, 0x61, 0xFF]; // indefinite text, "a", break
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_indefinite_array() {
        let buf = vec![0x9F, 0x01, 0xFF]; // indefinite array, 1, break
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_map() {
        let buf = vec![0xA1, 0x01, 0x02]; // map with 1 entry: 1 -> 2
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("map"));
    }

    #[test]
    fn rejects_tag() {
        let buf = vec![0xC1, 0x1A, 0x51, 0x4B, 0x67, 0xB0]; // tag 1 + uint
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("tag"));
    }

    #[test]
    fn rejects_float_f16() {
        let buf = vec![0xF9, 0x3C, 0x00]; // f16 = 1.0
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("float"));
    }

    #[test]
    fn rejects_float_f32() {
        let buf = vec![0xFA, 0x47, 0xC3, 0x50, 0x00]; // f32 = 100000.0
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("float"));
    }

    #[test]
    fn rejects_float_f64() {
        let buf = vec![0xFB, 0x7E, 0x37, 0xE4, 0x3C, 0x88, 0x00, 0x75, 0x9C]; // f64
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("float"));
    }

    #[test]
    fn rejects_negative_integer() {
        let buf = vec![0x20]; // -1
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("negative"));
    }

    #[test]
    fn rejects_undefined() {
        let buf = vec![0xF7]; // undefined (simple value 23)
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_break_code() {
        let buf = vec![0xFF]; // break outside indefinite
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn rejects_nesting_depth_5() {
        // 5 nested arrays: [[[[[1]]]]]
        let buf = vec![
            0x81, // array(1)
            0x81, // array(1)
            0x81, // array(1)
            0x81, // array(1)
            0x81, // array(1) — depth 5, exceeds MAX_NESTING=4
            0x01,
        ];
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("nesting"), "{}", err);
    }

    #[test]
    fn accepts_nesting_depth_4() {
        // 4 nested arrays: [[[[1]]]]
        let buf = vec![
            0x81, // array(1)
            0x81, // array(1)
            0x81, // array(1)
            0x81, // array(1) — depth 4 = MAX_NESTING
            0x01,
        ];
        assert!(decode(&buf).is_ok());
    }

    #[test]
    fn rejects_element_count_17() {
        // Array with 17 elements
        let mut buf = Vec::new();
        encode::array_head(&mut buf, 17);
        for i in 0..17u64 {
            encode::unsigned(&mut buf, i);
        }
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("element count"), "{}", err);
    }

    #[test]
    fn accepts_element_count_16() {
        let mut buf = Vec::new();
        encode::array_head(&mut buf, 16);
        for i in 0..16u64 {
            encode::unsigned(&mut buf, i);
        }
        assert!(decode(&buf).is_ok());
    }

    #[test]
    fn rejects_invalid_utf8() {
        // Text string with invalid UTF-8: 0x80 is a continuation byte
        let buf = vec![0x62, 0x80, 0x80]; // text(2), invalid bytes
        let err = decode(&buf).unwrap_err();
        assert!(err.to_string().contains("UTF-8"), "{}", err);
    }

    #[test]
    fn rejects_truncated_input() {
        // Array header says 3 elements but only 1 follows
        let mut buf = Vec::new();
        encode::array_head(&mut buf, 3);
        encode::unsigned(&mut buf, 1);
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn decode_sequence_head_valid() {
        let mut buf = Vec::new();
        encode::text(&mut buf, "rekindle identity rotation v1");
        encode::array_head(&mut buf, 2);
        encode::unsigned(&mut buf, 1);
        encode::unsigned(&mut buf, 2);

        let (domain, fields, consumed) = decode_sequence_head(&buf).unwrap();
        assert_eq!(domain, "rekindle identity rotation v1");
        assert_eq!(fields.len(), 2);
        assert_eq!(consumed, buf.len(), "must consume entire buffer when no trailing data");
    }

    #[test]
    fn decode_sequence_head_wrong_first_item() {
        let mut buf = Vec::new();
        encode::unsigned(&mut buf, 42); // not text
        encode::array_head(&mut buf, 1);
        encode::unsigned(&mut buf, 1);

        assert!(decode_sequence_head(&buf).is_err());
    }

    #[test]
    fn decode_sequence_head_wrong_second_item() {
        let mut buf = Vec::new();
        encode::text(&mut buf, "domain");
        encode::unsigned(&mut buf, 1); // not array

        assert!(decode_sequence_head(&buf).is_err());
    }
}
