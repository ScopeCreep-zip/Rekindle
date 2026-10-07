//! `serde(with = ...)` adapter encoding `Vec<u8>` as a base64 string.
//!
//! Used by every JSON wire type that carries bytes: `ChannelMessage` and
//! the community manifest entries (`rekindle-codec`), and the presence
//! row (`presence::MemberPresence`), whose route blob and signature
//! written as JSON number lists cost about 3.6 bytes per byte and pushed
//! the row past its 4112-byte registry slot (plan C7.15). It was declared
//! twice, verbatim, in `community/types.rs` and `channel_record/types.rs`,
//! and lives in Tier 1 so the presence row can use the same one.
//!
//! This is wire-visible: the daemon track's `ChannelMessage` copy simply
//! omitted the attribute, so it wrote a JSON number array where these
//! types write a base64 string, and messages written by one track failed
//! to parse on the other. Keeping one adapter is what makes "is this
//! field base64?" a single answerable question.

use base64::Engine;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    serializer.serialize_str(&b64)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(deserializer)?;
    base64::engine::general_purpose::STANDARD
        .decode(&s)
        .map_err(serde::de::Error::custom)
}

/// Bytes that serialize as a base64 string, for a field that is optional
/// (`Option<Base64Bytes>`), where a `with` module would take `&Option<_>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Base64Bytes(pub Vec<u8>);

impl serde::Serialize for Base64Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for Base64Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize(deserializer).map(Self)
    }
}
