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
    /// A friend-request token: 16 random bytes as 26 characters of
    /// unpadded lowercase RFC 4648 base32 (plan E2.5, D17).
    FriendRequestToken
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
/// Longest friend-invite id: a hyphenated UUID is 36.
pub const MAX_INVITE_ID_LEN: usize = 64;
/// Longest reaction: one emoji, which a ZWJ sequence with skin tones and
/// variation selectors spells in up to a dozen code points.
pub const MAX_REACTION_LEN: usize = 32;

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

/// The 32 bytes of a `VLD0:` key text's first segment: a public key's
/// (`VLD0:<key>`) or a record key's opaque key (`VLD0:<opaque>[:<enc>]`).
/// `None` for any other shape.
#[must_use]
pub fn vld0_key_bytes(s: &str) -> Option<[u8; 32]> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let rest = s.strip_prefix(RECORD_KIND_PREFIX)?;
    let mut segments = rest.split(':');
    let first = segments.next()?;
    check_b64url_segment(first, RECORD_KIND_PREFIX.len()).ok()?;
    if let Some(enc) = segments.next() {
        check_b64url_segment(enc, 0).ok()?;
    }
    if segments.next().is_some() {
        return None;
    }
    URL_SAFE_NO_PAD.decode(first).ok()?.try_into().ok()
}

/// The opaque key and the encryption key of a `VLD0:<opaque>:<enc>` record
/// key text, 32 bytes each. `None` for any other shape, a key with no
/// encryption key among them.
#[must_use]
pub fn vld0_record_key_parts(s: &str) -> Option<([u8; 32], [u8; 32])> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let opaque = vld0_key_bytes(s)?;
    let enc = s.strip_prefix(RECORD_KIND_PREFIX)?.split(':').nth(1)?;
    Some((opaque, URL_SAFE_NO_PAD.decode(enc).ok()?.try_into().ok()?))
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

/// Unpadded base32 length of 16 bytes.
pub const FRIEND_REQUEST_TOKEN_LEN: usize = 26;

/// Validate a friend-request token (26 of `a-z`, `2-7`). The last
/// character carries 2 padding bits, which must be zero, so every token
/// has exactly one spelling.
pub fn friend_request_token(s: &str) -> Result<FriendRequestToken, KeyFormatError> {
    if s.is_empty() {
        return Err(KeyFormatError::Empty);
    }
    if s.len() != FRIEND_REQUEST_TOKEN_LEN {
        return Err(KeyFormatError::BadLength {
            expected: FRIEND_REQUEST_TOKEN_LEN,
            got: s.len(),
        });
    }
    let value = |b: u8| match b {
        b'a'..=b'z' => Some(b - b'a'),
        b'2'..=b'7' => Some(b - b'2' + 26),
        _ => None,
    };
    for (index, b) in s.bytes().enumerate() {
        let Some(v) = value(b) else {
            return Err(KeyFormatError::BadChar { index });
        };
        if index == FRIEND_REQUEST_TOKEN_LEN - 1 && v & 0b11 != 0 {
            return Err(KeyFormatError::BadChar { index });
        }
    }
    Ok(FriendRequestToken(s.to_owned()))
}

/// Whether text may not carry `c`: a control character (`Cc`) or a hidden
/// format character (`Cf` but the joiners: bidi controls, zero-width
/// spaces, tags; [`crate::format_chars`]). The same rule holds for what a
/// user types (the IPC validation) and for what a peer sends (the 1:1
/// decoder, plan audit G6).
fn forbidden(c: char) -> bool {
    c.is_control() || crate::format_chars::is_hidden_format(c)
}

/// Validate a human-entered name: trimmed, 1..=`max` chars, no control or
/// hidden format characters. Returns the trimmed text.
pub fn name(s: &str, max: usize) -> Result<&str, KeyFormatError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(KeyFormatError::Empty);
    }
    let got = trimmed.chars().count();
    if got > max {
        return Err(KeyFormatError::TooLong { max, got });
    }
    match trimmed.chars().position(forbidden) {
        Some(index) => Err(KeyFormatError::BadChar { index }),
        None => Ok(trimmed),
    }
}

/// Validate free text a user typed — a topic, description, reason or
/// note: at most `max` chars, no control characters except newline and
/// tab, and no hidden format characters. Empty text is allowed; callers
/// that need content check for it.
pub fn text(s: &str, max: usize) -> Result<&str, KeyFormatError> {
    let got = s.chars().count();
    if got > max {
        return Err(KeyFormatError::TooLong { max, got });
    }
    match s
        .chars()
        .position(|c| forbidden(c) && c != '\n' && c != '\t')
    {
        Some(index) => Err(KeyFormatError::BadChar { index }),
        None => Ok(s),
    }
}

/// Validate a message body: [`text`] within [`MAX_MESSAGE_LEN`], and not
/// blank.
pub fn message_body(s: &str) -> Result<&str, KeyFormatError> {
    if s.trim().is_empty() {
        return Err(KeyFormatError::Empty);
    }
    text(s, MAX_MESSAGE_LEN)
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
#[path = "key_format_tests.rs"]
mod tests;
