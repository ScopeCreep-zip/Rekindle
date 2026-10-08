//! Validated identifier strings, shared by every boundary that accepts one
//! from outside the process: Tauri commands, the daemon IPC dispatcher,
//! deep links, and window labels.
//!
//! Each validator checks the exact shape the code produces:
//!
//! | Type | Shape | Produced by |
//! |---|---|---|
//! | [`RecordKeyStr`] | `VLD0:` + 43 base64url, optionally `:` + 43 base64url | veilid-core `RecordKey` Display (opaque key, optional encryption key) |
//! | [`PublicKeyHex`] / [`PseudonymHex`] | 64 lowercase hex | `hex::encode` of an Ed25519 public key |
//! | [`CallId`] / [`Hex16Id`] | 32 lowercase hex | `hex::encode` of 16 random bytes (call ids, channel/attachment ids, invite codes) |
//! | [`CommunityRef`] | a record key, or a 1–100-char name | daemon commands that accept a community by key or name |
//!
//! The newtypes hold the validated string, so code downstream takes a type
//! rather than re-checking a `&str`. Every type's alphabet is limited to
//! `[A-Za-z0-9:_-]` (names excepted), so validated values are safe in window
//! labels and URL query strings without escaping.

use std::fmt;

/// Why a string was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFormatError {
    Empty,
    BadPrefix,
    BadLength { expected: usize, got: usize },
    BadChar { index: usize },
    TooManySegments,
    TooLong { max: usize, got: usize },
}

impl fmt::Display for KeyFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::BadPrefix => f.write_str("wrong prefix"),
            Self::BadLength { expected, got } => write!(f, "length {got}, expected {expected}"),
            Self::BadChar { index } => write!(f, "invalid character at {index}"),
            Self::TooManySegments => f.write_str("too many segments"),
            Self::TooLong { max, got } => write!(f, "length {got}, max {max}"),
        }
    }
}

impl std::error::Error for KeyFormatError {}

macro_rules! validated {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

validated!(
    /// A Veilid DHT record key: `VLD0:<opaque>` or `VLD0:<opaque>:<encryption key>`.
    RecordKeyStr
);
validated!(
    /// An identity Ed25519 public key, 64 lowercase hex.
    PublicKeyHex
);
validated!(
    /// A per-community pseudonym public key, 64 lowercase hex.
    PseudonymHex
);
validated!(
    /// A call id, 32 lowercase hex (16 random bytes).
    CallId
);
validated!(
    /// A 16-byte random id as 32 lowercase hex: channel ids, attachment ids,
    /// invite codes.
    Hex16Id
);

impl PublicKeyHex {
    /// The 32 key bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        decode_lower_hex_32(&self.0)
    }
}

impl PseudonymHex {
    /// The 32 key bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        decode_lower_hex_32(&self.0)
    }
}

/// Decode 64 lowercase hex digits, already checked by `check_lower_hex`.
fn decode_lower_hex_32(s: &str) -> [u8; 32] {
    let nibble = |b: u8| {
        if b.is_ascii_digit() {
            b - b'0'
        } else {
            b - b'a' + 10
        }
    };
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(s.as_bytes().chunks_exact(2)) {
        *byte = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    out
}

/// The only crypto kind Rekindle creates records with.
const RECORD_KIND_PREFIX: &str = "VLD0:";
/// Unpadded base64url length of a 32-byte value.
const B64URL_32: usize = 43;
/// Longest community/channel name the daemon accepts.
pub const MAX_NAME_LEN: usize = 100;
/// Longest message body: Discord's message `content` cap (up to 2000
/// characters).
pub const MAX_MESSAGE_LEN: usize = 2000;
/// Longest channel topic: Discord's channel `topic` cap (0-1024
/// characters).
pub const MAX_TOPIC_LEN: usize = 1024;
/// Longest community description: Telegram's group/channel description
/// cap (`setChatDescription`, 0-255 characters).
pub const MAX_DESCRIPTION_LEN: usize = 255;
/// Longest short note — a moderation reason or a friend-request message:
/// Discord's audit-log `reason` cap (1-512 characters).
pub const MAX_NOTE_LEN: usize = 512;
/// Longest status message: Slack's custom status text (`status_text`, up
/// to 100 characters).
pub const MAX_STATUS_LEN: usize = 100;
/// Longest game-server address: a DNS name (RFC 1035 §2.3.4, at most 255
/// octets on the wire, so 253 characters as text) plus `:` and a
/// five-digit port.
pub const MAX_SERVER_ADDRESS_LEN: usize = 259;

fn is_b64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn check_b64url_segment(seg: &str, offset: usize) -> Result<(), KeyFormatError> {
    if seg.len() != B64URL_32 {
        return Err(KeyFormatError::BadLength {
            expected: B64URL_32,
            got: seg.len(),
        });
    }
    match seg.bytes().position(|b| !is_b64url(b)) {
        Some(i) => Err(KeyFormatError::BadChar { index: offset + i }),
        None => Ok(()),
    }
}

fn check_lower_hex(s: &str, len: usize) -> Result<(), KeyFormatError> {
    if s.is_empty() {
        return Err(KeyFormatError::Empty);
    }
    if s.len() != len {
        return Err(KeyFormatError::BadLength {
            expected: len,
            got: s.len(),
        });
    }
    match s
        .bytes()
        .position(|b| !(b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    {
        Some(index) => Err(KeyFormatError::BadChar { index }),
        None => Ok(()),
    }
}

/// Validate a Veilid record key (`VLD0:` + 43 base64url, optional `:` + 43).
pub fn record_key(s: &str) -> Result<RecordKeyStr, KeyFormatError> {
    if s.is_empty() {
        return Err(KeyFormatError::Empty);
    }
    let rest = s
        .strip_prefix(RECORD_KIND_PREFIX)
        .ok_or(KeyFormatError::BadPrefix)?;
    let mut segments = rest.split(':');
    let opaque = segments.next().unwrap_or_default();
    check_b64url_segment(opaque, RECORD_KIND_PREFIX.len())?;
    if let Some(enc) = segments.next() {
        check_b64url_segment(enc, RECORD_KIND_PREFIX.len() + opaque.len() + 1)?;
    }
    if segments.next().is_some() {
        return Err(KeyFormatError::TooManySegments);
    }
    Ok(RecordKeyStr(s.to_owned()))
}

/// Validate an identity public key (64 lowercase hex).
pub fn public_key_hex(s: &str) -> Result<PublicKeyHex, KeyFormatError> {
    check_lower_hex(s, 64)?;
    Ok(PublicKeyHex(s.to_owned()))
}

/// Validate a community pseudonym key (64 lowercase hex).
pub fn pseudonym_hex(s: &str) -> Result<PseudonymHex, KeyFormatError> {
    check_lower_hex(s, 64)?;
    Ok(PseudonymHex(s.to_owned()))
}

/// Validate a call id (32 lowercase hex).
pub fn call_id(s: &str) -> Result<CallId, KeyFormatError> {
    check_lower_hex(s, 32)?;
    Ok(CallId(s.to_owned()))
}

/// Validate a 16-byte hex id (32 lowercase hex).
pub fn hex16_id(s: &str) -> Result<Hex16Id, KeyFormatError> {
    check_lower_hex(s, 32)?;
    Ok(Hex16Id(s.to_owned()))
}

/// Validate a human-entered name: trimmed, 1..=`max` chars, no control
/// characters. Returns the trimmed text.
pub fn name(s: &str, max: usize) -> Result<&str, KeyFormatError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(KeyFormatError::Empty);
    }
    let got = trimmed.chars().count();
    if got > max {
        return Err(KeyFormatError::TooLong { max, got });
    }
    match trimmed.chars().position(char::is_control) {
        Some(index) => Err(KeyFormatError::BadChar { index }),
        None => Ok(trimmed),
    }
}

/// Validate free text a user typed — a topic, description, reason or
/// note: at most `max` chars, no control characters except newline and
/// tab. Empty text is allowed; callers that need content check for it.
pub fn text(s: &str, max: usize) -> Result<&str, KeyFormatError> {
    let got = s.chars().count();
    if got > max {
        return Err(KeyFormatError::TooLong { max, got });
    }
    match s
        .chars()
        .position(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Some(index) => Err(KeyFormatError::BadChar { index }),
        None => Ok(s),
    }
}

/// A channel named by its 32-hex id or by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelRef {
    Id(Hex16Id),
    Name(String),
}

/// Validate a channel reference: a 32-hex id if it is one, otherwise a
/// name per [`name`].
pub fn channel_ref(s: &str) -> Result<ChannelRef, KeyFormatError> {
    if let Ok(id) = hex16_id(s) {
        return Ok(ChannelRef::Id(id));
    }
    name(s, MAX_NAME_LEN).map(|n| ChannelRef::Name(n.to_owned()))
}

/// A community named by its governance record key or by its display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommunityRef {
    Key(RecordKeyStr),
    Name(String),
}

/// Validate a community reference: a record key if it has the record-key
/// prefix, otherwise a name per [`name`].
pub fn community_ref(s: &str) -> Result<CommunityRef, KeyFormatError> {
    if s.starts_with(RECORD_KIND_PREFIX) {
        return record_key(s).map(CommunityRef::Key);
    }
    name(s, MAX_NAME_LEN).map(|n| CommunityRef::Name(n.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
    const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";

    #[test]
    fn hex_keys_decode_to_bytes() {
        let bytes = public_key_hex(HEX64).unwrap().to_bytes();
        assert_eq!(bytes[..3], [0x3f, 0x1a, 0x9c]);
        assert_eq!(bytes[31], 0x1a);
        assert_eq!(pseudonym_hex(HEX64).unwrap().to_bytes(), bytes);
    }

    #[test]
    fn record_keys() {
        assert!(record_key(&format!("VLD0:{B64}")).is_ok());
        assert!(record_key(&format!("VLD0:{B64}:{B64}")).is_ok());
        assert_eq!(record_key(""), Err(KeyFormatError::Empty));
        assert_eq!(
            record_key(&format!("XXX0:{B64}")),
            Err(KeyFormatError::BadPrefix)
        );
        assert_eq!(
            record_key(&format!("VLD0:{B64}:{B64}:{B64}")),
            Err(KeyFormatError::TooManySegments)
        );
        assert!(matches!(
            record_key(&format!("VLD0:{}", &B64[..42])),
            Err(KeyFormatError::BadLength {
                expected: 43,
                got: 42
            })
        ));
        assert!(matches!(
            record_key(&format!("VLD0:{B64}x")),
            Err(KeyFormatError::BadLength { got: 44, .. })
        ));
        let bad = format!("VLD0:{}/", &B64[..42]);
        assert!(matches!(
            record_key(&bad),
            Err(KeyFormatError::BadChar { index: 47 })
        ));
        assert!(record_key(&format!("VLD0:{}\0", &B64[..42])).is_err());
    }

    #[test]
    fn hex_ids() {
        assert!(public_key_hex(HEX64).is_ok());
        assert!(pseudonym_hex(HEX64).is_ok());
        assert!(public_key_hex(&HEX64.to_uppercase()).is_err());
        assert!(public_key_hex(&HEX64[..63]).is_err());
        assert!(call_id(&HEX64[..32]).is_ok());
        assert!(hex16_id(&HEX64[..32]).is_ok());
        assert!(call_id(&"é".repeat(16)).is_err());
        assert_eq!(call_id(""), Err(KeyFormatError::Empty));
        assert!(call_id("6f9619ff-8b86-d011-b42d-00cf4fc964ff").is_err());
    }

    #[test]
    fn names_and_community_refs() {
        assert_eq!(name("  Gamers  ", 100), Ok("Gamers"));
        assert_eq!(name("   ", 100), Err(KeyFormatError::Empty));
        assert!(matches!(
            name(&"a".repeat(101), 100),
            Err(KeyFormatError::TooLong { .. })
        ));
        assert!(name("bad\u{7}name", 100).is_err());
        assert_eq!(
            community_ref(&format!("VLD0:{B64}")),
            Ok(CommunityRef::Key(RecordKeyStr(format!("VLD0:{B64}"))))
        );
        assert_eq!(
            community_ref("Gamers"),
            Ok(CommunityRef::Name("Gamers".into()))
        );
        assert!(community_ref("VLD0:short").is_err());
    }

    #[test]
    fn text_counts_chars_and_allows_newlines() {
        assert_eq!(text("", 4), Ok(""));
        assert_eq!(text("a\nb\tc", 5), Ok("a\nb\tc"));
        assert!(text(&"é".repeat(4), 4).is_ok());
        assert_eq!(
            text(&"é".repeat(5), 4),
            Err(KeyFormatError::TooLong { max: 4, got: 5 })
        );
        assert_eq!(text("ab\x1b", 8), Err(KeyFormatError::BadChar { index: 2 }));
    }

    #[test]
    fn channel_refs() {
        let id = "0123456789abcdef0123456789abcdef";
        assert!(matches!(channel_ref(id), Ok(ChannelRef::Id(_))));
        assert_eq!(
            channel_ref(" general "),
            Ok(ChannelRef::Name("general".into()))
        );
        assert!(channel_ref("").is_err());
    }
}
