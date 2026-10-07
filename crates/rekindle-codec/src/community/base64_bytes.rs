//! `serde(with = ...)` adapter encoding `Vec<u8>` as a base64 string.
//!
//! Used by the DHT record types that carry ciphertext — `ChannelMessage`
//! and the community manifest entries. It was declared twice, verbatim,
//! in `community/types.rs` and `channel_record/types.rs`.
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
