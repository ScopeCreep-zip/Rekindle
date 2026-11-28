//! The identity archive (plan D1): the registry entry plus the vault and
//! its header, which the vault's own passphrase format already seals. No
//! new cryptography: an archive is only as open as the vault inside it.
//!
//! Layout: `b"RKIA"`, version `1`, then three length-prefixed parts
//! (u32 little-endian): the entry as JSON, the header, the vault.

use rekindle_db::registry::IdentityEntry;

use super::IdentityError;

const ARCHIVE_MAGIC: &[u8; 4] = b"RKIA";
const ARCHIVE_VERSION: u8 = 1;

/// An identity archive's parts.
pub struct Archive {
    pub entry: IdentityEntry,
    pub header: Vec<u8>,
    pub vault: Vec<u8>,
}

fn corrupt(what: &str) -> IdentityError {
    IdentityError::Corrupt(format!("identity archive: {what}"))
}

fn push_part(out: &mut Vec<u8>, part: &[u8]) -> Result<(), IdentityError> {
    let len = u32::try_from(part.len()).map_err(|_| corrupt("a part is over 4 GiB"))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(part);
    Ok(())
}

impl Archive {
    /// The archive's bytes.
    ///
    /// # Errors
    /// The entry does not serialize, or a part is too large.
    pub fn encode(&self) -> Result<Vec<u8>, IdentityError> {
        let entry = serde_json::to_vec(&self.entry).map_err(|e| corrupt(&e.to_string()))?;
        let mut out =
            Vec::with_capacity(5 + 12 + entry.len() + self.header.len() + self.vault.len());
        out.extend_from_slice(ARCHIVE_MAGIC);
        out.push(ARCHIVE_VERSION);
        push_part(&mut out, &entry)?;
        push_part(&mut out, &self.header)?;
        push_part(&mut out, &self.vault)?;
        Ok(out)
    }

    /// Parse an archive.
    ///
    /// # Errors
    /// [`IdentityError::Corrupt`]: wrong magic or version, a truncated part,
    /// trailing bytes, or an entry that does not parse.
    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        let rest = bytes
            .strip_prefix(ARCHIVE_MAGIC.as_slice())
            .ok_or_else(|| corrupt("not an identity archive"))?;
        let (&version, mut rest) = rest.split_first().ok_or_else(|| corrupt("truncated"))?;
        if version != ARCHIVE_VERSION {
            return Err(corrupt("unknown version"));
        }
        let mut parts = Vec::with_capacity(3);
        for _ in 0..3 {
            let (len, tail) = rest
                .split_first_chunk::<4>()
                .ok_or_else(|| corrupt("truncated"))?;
            let len = u32::from_le_bytes(*len) as usize;
            if tail.len() < len {
                return Err(corrupt("truncated"));
            }
            let (part, tail) = tail.split_at(len);
            parts.push(part.to_vec());
            rest = tail;
        }
        if !rest.is_empty() {
            return Err(corrupt("trailing bytes"));
        }
        let vault = parts.pop().unwrap_or_default();
        let header = parts.pop().unwrap_or_default();
        let entry = serde_json::from_slice(&parts.pop().unwrap_or_default())
            .map_err(|e| corrupt(&e.to_string()))?;
        Ok(Self {
            entry,
            header,
            vault,
        })
    }
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
