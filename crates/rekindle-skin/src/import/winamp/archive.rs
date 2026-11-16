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
//!
//! The end record and central headers are `crate::package::zip`'s; the
//! local header is read by this importer's own legacy reader
//! ([`local_data`]), because legacy archives carry the data-descriptor flag
//! (APPNOTE 6.3.10 §4.4.4 bit 3: the local CRC and sizes are zero and a
//! descriptor follows the data). The central directory is authoritative for
//! the sizes and CRC; the legacy limits and ratio above still bound them.
//! The `.rkskin` package reader stays strict and refuses bit 3.

use rekindle_types::dto::skin::SkinRule;

use super::super::{ImportError, Tree};
use crate::limits::MAX_ENTRIES;
use crate::package::zip::{self, FLAG_DATA_DESCRIPTOR, FLAG_ENCRYPTED};

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
const LOCAL_SIG: u32 = 0x0403_4b50;
const LOCAL_LEN: usize = 30;

pub(crate) fn refused(entry: &str, detail: impl Into<String>) -> ImportError {
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
    let range = local_data(bytes, name, h, directory_start)?;
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

fn le(bytes: &[u8], at: usize, n: usize) -> Option<u32> {
    let s = bytes.get(at..at.checked_add(n)?)?;
    Some(s.iter().rev().fold(0, |v, b| v << 8 | u32::from(*b)))
}

/// The legacy local header: its signature, flags, method and name repeat
/// the central record; its CRC and sizes repeat it too, or are zero when
/// the data-descriptor flag is set (the central record is authoritative).
/// Returns the entry's data range, which must end before the central
/// directory.
fn local_data(
    bytes: &[u8],
    name: &str,
    c: &zip::Central<'_>,
    directory_start: usize,
) -> Result<std::ops::Range<usize>, ImportError> {
    let at = c.local_offset;
    let bad = || refused(name, "local header disagrees with the central directory");
    let field = |off: usize, n: usize| le(bytes, at + off, n).ok_or_else(bad);
    if field(0, 4)? != LOCAL_SIG {
        return Err(refused(name, "bad local header signature"));
    }
    if field(6, 2)? != u32::from(c.flags) || field(8, 2)? != u32::from(c.method) {
        return Err(bad());
    }
    let descriptor = c.flags & FLAG_DATA_DESCRIPTOR != 0;
    for (off, central) in [(14, c.crc), (18, c.compressed), (22, c.uncompressed)] {
        let local = field(off, 4)?;
        if local != central && !(descriptor && local == 0) {
            return Err(bad());
        }
    }
    let name_len = usize::try_from(field(26, 2)?).map_err(|_| bad())?;
    let extra_len = usize::try_from(field(28, 2)?).map_err(|_| bad())?;
    let name_start = at + LOCAL_LEN;
    if bytes.get(name_start..name_start + name_len) != Some(c.name) {
        return Err(bad());
    }
    let start = name_start + name_len + extra_len;
    let packed = usize::try_from(c.compressed).map_err(|_| bad())?;
    let end = start
        .checked_add(packed)
        .filter(|&end| end <= directory_start)
        .ok_or_else(|| refused(name, "entry data runs into the central directory"))?;
    Ok(start..end)
}
