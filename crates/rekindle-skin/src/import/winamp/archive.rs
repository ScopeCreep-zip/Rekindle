//! The legacy `.wsz` archive, read in memory with the package walk's own
//! strict zip records (`crate::package::zip`) under the legacy limits of
//! the plan's §6 ledger: the archive ≤ 32 MiB, ≤ 64 MiB uncompressed, each
//! entry's ratio ≤ 100:1. Stored (0) and deflate (8) are the only methods,
//! and deflate is inflated only here, bounded by the entry's declared size.
//!
//! Every entry's declared sizes are checked, but only the files the
//! importer reads ([`WANTED`]) are inflated; every other entry (cursors,
//! `REGION.TXT`, `TEXT.BMP`, `WINAMPMB.TXT`, …) is never read past its name.
//! Webamp matches files by base name, case-insensitively, in any folder
//! (`skinParser.js:10-49`); so does this, and two wanted files with one
//! base name refuse the archive (Webamp's "last wins").

use rekindle_types::dto::skin::SkinRule;

use super::super::{ImportError, Tree};
use crate::limits::MAX_ENTRIES;
use crate::package::zip::{self, FLAG_ENCRYPTED};

/// A legacy archive's bytes.
pub const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
/// Every entry's declared uncompressed bytes together.
pub const MAX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
/// An entry's uncompressed bytes per compressed byte.
pub const MAX_RATIO: u64 = 100;

/// The files the importer reads, by lower-case base name.
pub const WANTED: [&str; 4] = ["gen.bmp", "genex.bmp", "titlebar.bmp", "pledit.txt"];

const STORED: u16 = 0;
const DEFLATE: u16 = 8;

fn refused(entry: &str, detail: impl Into<String>) -> ImportError {
    ImportError::Archive {
        entry: entry.to_owned(),
        detail: detail.into(),
    }
}

fn base_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The archive's [`WANTED`] files, keyed by lower-case base name, and the
/// names of every other entry (never inflated).
///
/// # Errors
/// [`ImportError::Archive`] past a limit, for an encrypted or otherwise
/// compressed wanted entry, a bad CRC or a duplicate wanted name;
/// [`ImportError::Refused`] for a malformed zip record.
pub fn unpack(bytes: &[u8]) -> Result<(Tree, Vec<String>), ImportError> {
    let size = bytes.len() as u64;
    if size > MAX_ARCHIVE_BYTES {
        return Err(refused(
            "archive",
            format!("{size} bytes is past {MAX_ARCHIVE_BYTES}"),
        ));
    }
    let dir = zip::directory(bytes).map_err(ImportError::Refused)?;
    if dir.count as u64 > MAX_ENTRIES {
        return Err(refused(
            "archive",
            format!("{} entries is past {MAX_ENTRIES}", dir.count),
        ));
    }
    let mut tree = Tree::new();
    let mut ignored = Vec::new();
    let mut total = 0u64;
    let mut at = dir.offset;
    for _ in 0..dir.count {
        let (h, next) = zip::central(bytes, at).map_err(ImportError::Refused)?;
        at = next;
        let name = String::from_utf8_lossy(h.name).into_owned();
        let (packed, unpacked) = (u64::from(h.compressed), u64::from(h.uncompressed));
        total += unpacked;
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(refused(
                &name,
                format!("{total} bytes uncompressed is past {MAX_UNCOMPRESSED_BYTES}"),
            ));
        }
        if unpacked > packed.max(1) * MAX_RATIO {
            return Err(refused(
                &name,
                format!("ratio {unpacked}:{packed} is past {MAX_RATIO}:1"),
            ));
        }
        let base = base_name(&name);
        if !WANTED.contains(&base.as_str()) || name.ends_with('/') {
            ignored.push(name);
            continue;
        }
        if tree.contains_key(&base) {
            return Err(refused(&name, "a second file with this base name"));
        }
        let data = inflate(bytes, &name, &h, dir.offset)?;
        tree.insert(base, data);
    }
    if at != dir.end {
        return Err(ImportError::Refused(zip::malformed(
            "archive",
            "central directory size disagrees with its records",
        )));
    }
    Ok((tree, ignored))
}

fn inflate(
    bytes: &[u8],
    name: &str,
    h: &zip::Central<'_>,
    directory_start: usize,
) -> Result<Vec<u8>, ImportError> {
    if h.flags & FLAG_ENCRYPTED != 0 {
        return Err(ImportError::Refused(
            rekindle_types::dto::skin::SkinError::new(name, SkinRule::EncryptedEntry),
        ));
    }
    let range = zip::local_data(bytes, name, h, directory_start).map_err(ImportError::Refused)?;
    let raw = &bytes[range];
    let size = usize::try_from(h.uncompressed).unwrap_or(usize::MAX);
    let data = match h.method {
        STORED => raw.to_vec(),
        DEFLATE => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size)
            .map_err(|e| refused(name, format!("deflate: {:?}", e.status)))?,
        method => return Err(refused(name, format!("compression method {method}"))),
    };
    if data.len() != size {
        return Err(refused(name, "size disagrees with the central directory"));
    }
    if crc32fast::hash(&data) != h.crc {
        return Err(refused(name, "CRC-32 mismatch"));
    }
    Ok(data)
}
