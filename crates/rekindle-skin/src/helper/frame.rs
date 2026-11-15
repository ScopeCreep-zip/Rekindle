//! The helper's answer on stdout: one framed, versioned record.
//!
//! ```text
//! magic "RKSH" ‖ version u8 (1) ‖ tag u8 ‖ length u32 LE ‖ payload
//! tag 0, canonical: BLAKE3 hash (32 bytes) ‖ the canonical package
//! tag 1, refused:   the SkinError as JSON
//! ```
//!
//! The decoder takes the whole stream and refuses anything else: another
//! magic or version, an unknown tag, a length that is not the rest of the
//! stream, a package over the size limit, a hash that is not the BLAKE3 of
//! the package, or a refusal that does not parse.

use rekindle_types::dto::skin::SkinError;

use crate::limits::MAX_PACKAGE_BYTES;
use crate::output::Canonical;

/// The first four bytes of every answer.
pub const MAGIC: [u8; 4] = *b"RKSH";
/// The frame version this crate writes and reads.
pub const VERSION: u8 = 1;
const TAG_CANONICAL: u8 = 0;
const TAG_REFUSED: u8 = 1;
const HEADER: usize = 4 + 1 + 1 + 4;
/// A refusal's JSON never comes near this.
const MAX_REFUSAL_BYTES: usize = 64 * 1024;
/// The longest answer a valid helper writes: the header, the hash and a
/// package at the size limit.
pub const MAX_FRAME_BYTES: u64 = 10 + 32 + MAX_PACKAGE_BYTES;

/// What the helper answered.
pub type Answer = Result<Canonical, SkinError>;

fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(tag);
    // A payload is at most MAX_FRAME_BYTES, far under u32::MAX.
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// `answer` as one frame.
#[must_use]
pub fn encode(answer: &Answer) -> Vec<u8> {
    match answer {
        Ok(c) => {
            let mut payload = Vec::with_capacity(32 + c.bytes.len());
            payload.extend_from_slice(&c.hash);
            payload.extend_from_slice(&c.bytes);
            frame(TAG_CANONICAL, &payload)
        }
        Err(e) => {
            // A SkinError is plain strings and numbers: it always serialises.
            let json = serde_json::to_vec(e).unwrap_or_default();
            frame(TAG_REFUSED, &json)
        }
    }
}

/// Reads one whole answer from `stream`.
///
/// # Errors
/// What is wrong with the stream, as text for [`super::HelperError::Malformed`].
pub fn decode(stream: &[u8]) -> Result<Answer, String> {
    let (header, payload) = stream
        .split_at_checked(HEADER)
        .ok_or_else(|| format!("{} bytes, shorter than the header", stream.len()))?;
    if header[..4] != MAGIC {
        return Err("not a helper answer (magic)".to_owned());
    }
    if header[4] != VERSION {
        return Err(format!("frame version {}, expected {VERSION}", header[4]));
    }
    let len = u32::from_le_bytes([header[6], header[7], header[8], header[9]]);
    if usize::try_from(len).ok() != Some(payload.len()) {
        return Err(format!(
            "length {len}, but {} payload bytes follow",
            payload.len()
        ));
    }
    match header[5] {
        TAG_CANONICAL => canonical(payload).map(Ok),
        TAG_REFUSED => {
            if payload.len() > MAX_REFUSAL_BYTES {
                return Err(format!("a {}-byte refusal", payload.len()));
            }
            serde_json::from_slice::<SkinError>(payload)
                .map(Err)
                .map_err(|e| format!("refusal does not parse: {e}"))
        }
        tag => Err(format!("unknown tag {tag}")),
    }
}

fn canonical(payload: &[u8]) -> Result<Canonical, String> {
    let (hash, bytes) = payload
        .split_first_chunk::<32>()
        .ok_or_else(|| "canonical answer without a hash".to_owned())?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(format!("a {}-byte package", bytes.len()));
    }
    if blake3::hash(bytes).as_bytes() != hash {
        return Err("hash is not the BLAKE3 of the package".to_owned());
    }
    Ok(Canonical {
        bytes: bytes.to_vec(),
        hash: *hash,
    })
}
