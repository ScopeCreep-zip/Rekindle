//! Validator steps 1: the outer size and the strict zip walk (plan E7.7.1).
//!
//! The package is read in memory and never extracted. Every entry is
//! stored (method 0), so nothing is inflated and no decompression bomb is
//! possible; each entry's data is a slice of the input, checked against
//! its CRC-32. Refused: encrypted entries, symbolic links and other special
//! files, backslashes, absolute paths, `..`, empty or `.` components,
//! anything outside the layout, duplicate names and names that collide by
//! case or Unicode normalisation (Pidgin CVE-2014-3697; Webamp's "last
//! wins"), entries whose data overlaps, and archives past the
//! [`crate::limits`].

mod path;
mod zip;

use std::collections::{BTreeMap, HashMap, HashSet};

use rekindle_types::dto::skin::{SkinError, SkinRule};

pub use path::{is_root_json, LAYOUT_DIRS, MANIFEST};

use crate::limits::{MAX_ENTRIES, MAX_PACKAGE_BYTES};
use zip::{malformed, Central, FLAG_DATA_DESCRIPTOR, FLAG_ENCRYPTED, FLAG_UTF8};

/// Unix file types in the high half of the external attributes when the
/// "made by" host is Unix (APPNOTE §4.4.2, §4.4.15).
const HOST_UNIX: u16 = 3;
const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;
/// `FILE_ATTRIBUTE_REPARSE_POINT` in the low (MS-DOS) attribute half.
const DOS_REPARSE_POINT: u32 = 0x400;

/// A package that passed the walk: its files by name, each a slice of the
/// input bytes. Directory entries are checked and then dropped.
#[derive(Debug)]
pub struct Package<'a> {
    files: BTreeMap<String, &'a [u8]>,
}

impl<'a> Package<'a> {
    /// The data of the file `name`, if the package holds it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'a [u8]> {
        self.files.get(name).copied()
    }
}

/// Walks `bytes` as a skin package.
///
/// # Errors
/// The first rule the package breaks, naming the entry.
pub fn read(bytes: &[u8]) -> Result<Package<'_>, SkinError> {
    let size = bytes.len() as u64;
    if size > MAX_PACKAGE_BYTES {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::PackageTooLarge {
                size,
                max: MAX_PACKAGE_BYTES,
            },
        ));
    }
    let dir = zip::directory(bytes)?;
    if dir.count as u64 > MAX_ENTRIES {
        return Err(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::TooManyEntries {
                count: dir.count as u64,
                max: MAX_ENTRIES,
            },
        ));
    }
    let mut walk = Walk::default();
    let mut at = dir.offset;
    for _ in 0..dir.count {
        let (header, next) = zip::central(bytes, at)?;
        walk.entry(bytes, &header, dir.offset)?;
        at = next;
    }
    if at != dir.end {
        return Err(malformed(
            SkinError::PACKAGE,
            "central directory size disagrees with its records",
        ));
    }
    walk.check_overlaps()?;
    if !walk.files.contains_key(MANIFEST) {
        return Err(SkinError::new(MANIFEST, SkinRule::MissingManifest));
    }
    Ok(Package { files: walk.files })
}

#[derive(Default)]
struct Walk<'a> {
    files: BTreeMap<String, &'a [u8]>,
    names: HashSet<String>,
    keys: HashMap<String, String>,
    /// (local header offset, data end, name) of every entry.
    spans: Vec<(usize, usize, String)>,
}

impl<'a> Walk<'a> {
    fn entry(
        &mut self,
        bytes: &'a [u8],
        h: &Central<'_>,
        directory_start: usize,
    ) -> Result<(), SkinError> {
        let name = entry_name(h)?;
        let refuse = |rule| SkinError::new(name.as_str(), rule);
        if h.flags & FLAG_ENCRYPTED != 0 {
            return Err(refuse(SkinRule::EncryptedEntry));
        }
        if h.method != 0 {
            return Err(refuse(SkinRule::CompressedEntry { method: h.method }));
        }
        if h.flags & FLAG_DATA_DESCRIPTOR != 0 || h.compressed != h.uncompressed {
            return Err(malformed(&name, "stored entry sizes are not exact"));
        }
        check_file_type(h).map_err(refuse)?;
        let is_dir = name.ends_with('/');
        let path = name.strip_suffix('/').unwrap_or(&name);
        path::check_shape(path).map_err(refuse)?;
        path::check_layout(path, is_dir).map_err(refuse)?;
        if !self.names.insert(path.to_owned()) {
            return Err(refuse(SkinRule::DuplicateName));
        }
        let key = path::collision_key(path);
        if let Some(other) = self.keys.get(&key) {
            return Err(refuse(SkinRule::CaseCollision {
                other: other.clone(),
            }));
        }
        self.keys.insert(key, path.to_owned());
        let range = zip::local_data(bytes, &name, h, directory_start)?;
        let data = &bytes[range.clone()];
        if is_dir && !data.is_empty() {
            return Err(malformed(&name, "directory entry with data"));
        }
        if crc32fast::hash(data) != h.crc {
            return Err(refuse(SkinRule::CrcMismatch));
        }
        self.spans.push((h.local_offset, range.end, name.clone()));
        if !is_dir {
            self.files.insert(name, data);
        }
        Ok(())
    }

    /// Each entry's local header starts at or after the previous entry's
    /// data end, so no byte belongs to two entries.
    fn check_overlaps(&mut self) -> Result<(), SkinError> {
        self.spans.sort_unstable();
        for pair in self.spans.windows(2) {
            if pair[1].0 < pair[0].1 {
                return Err(malformed(&pair[1].2, "entry overlaps another entry"));
            }
        }
        Ok(())
    }
}

/// The entry's name as UTF-8; a non-ASCII name must carry the UTF-8 flag
/// (otherwise it is CP437 and reads differently per tool).
fn entry_name(h: &Central<'_>) -> Result<String, SkinError> {
    let name = std::str::from_utf8(h.name)
        .map_err(|_| malformed(SkinError::PACKAGE, "entry name is not UTF-8"))?;
    if !name.is_ascii() && h.flags & FLAG_UTF8 == 0 {
        return Err(malformed(name, "non-ASCII name without the UTF-8 flag"));
    }
    if name.is_empty() {
        return Err(malformed(SkinError::PACKAGE, "empty entry name"));
    }
    Ok(name.to_owned())
}

/// An entry must not be a Windows reparse point (a junction or symbolic
/// link), and a Unix-made entry must be a regular file or a directory.
fn check_file_type(h: &Central<'_>) -> Result<(), SkinRule> {
    if h.external_attrs & DOS_REPARSE_POINT != 0 {
        return Err(SkinRule::Symlink);
    }
    if h.made_by >> 8 != HOST_UNIX {
        return Ok(());
    }
    let mode = h.external_attrs >> 16;
    match mode & S_IFMT {
        0 | S_IFREG | S_IFDIR => Ok(()),
        S_IFLNK => Err(SkinRule::Symlink),
        _ => Err(SkinRule::SpecialFile { mode }),
    }
}
