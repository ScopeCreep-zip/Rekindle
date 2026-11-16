//! A strict tar reader (POSIX.1-1988 ustar, IEEE Std 1003.1 `pax` §
//! "ustar Interchange Format") over a stream already in memory, for the
//! Pidgin `.tgz` importer only.
//!
//! Every 512-byte header's checksum is verified (unsigned sum, the
//! checksum field read as spaces). The magic is POSIX `ustar\0` `00` (its
//! `prefix` joins the name) or old GNU `ustar  \0` (GNU tar's default
//! format, which most distributed themes were made with; its `prefix`
//! bytes are not a name and are not read). Only regular files (`0`, NUL)
//! and directories (`5`) are taken; links, devices, FIFOs, contiguous
//! files, pax extended headers (`x`, `g`) and GNU long names (`L`, `K`) or
//! sparse files refuse the archive. Numbers are octal only (GNU base-256
//! refused). A name holds no NUL before its padding, is UTF-8 and a plain
//! relative path (no `/` start, `.`, `..`, empty part, `\` or `:`); no path
//! appears twice, and no file is another entry's parent. Every entry lies
//! under one top-level directory, which is stripped from the output, so the
//! tree matches a theme directory read from disk.
//!
//! Sizes are checked before anything is copied: each file against
//! [`MAX_THEME_FILE_BYTES`], all files together against the total cap, the
//! entry count against [`MAX_ENTRIES`]. The archive ends with two zero
//! blocks and only zero padding after them.

use std::collections::BTreeMap;

use super::super::winamp::archive::refused;
use super::super::{ImportError, Tree};
use super::map::plain_path;
use crate::limits::{MAX_ENTRIES, MAX_PACKAGE_BYTES};

/// One file's bytes: no more than a whole package may hold.
pub const MAX_THEME_FILE_BYTES: u64 = MAX_PACKAGE_BYTES;

const BLOCK: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
}

struct Header {
    path: String,
    kind: Kind,
    size: u64,
}

/// `field` up to its first NUL; every byte after that must be NUL.
fn text<'a>(entry: &str, field: &'a [u8], what: &str) -> Result<&'a [u8], ImportError> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    if field[end..].iter().any(|b| *b != 0) {
        return Err(refused(entry, format!("a NUL inside the {what}")));
    }
    Ok(&field[..end])
}

/// An octal number: optional leading spaces, digits, then spaces or NULs.
fn octal(entry: &str, field: &[u8], what: &str) -> Result<u64, ImportError> {
    let bad = || refused(entry, format!("{what} is not an octal number"));
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        return Err(refused(entry, format!("{what} is base-256 (GNU)")));
    }
    let digits = field.iter().skip_while(|b| **b == b' ');
    let mut value = 0u64;
    let mut seen = 0;
    let mut ended = false;
    for &b in digits {
        match b {
            b'0'..=b'7' if !ended => {
                value = value.checked_mul(8).ok_or_else(bad)? + u64::from(b - b'0');
                seen += 1;
            }
            b' ' | 0 if seen > 0 => ended = true,
            _ => return Err(bad()),
        }
    }
    if seen == 0 {
        return Err(bad());
    }
    Ok(value)
}

fn kind(entry: &str, flag: u8) -> Result<Kind, ImportError> {
    let what = match flag {
        b'0' | 0 => return Ok(Kind::File),
        b'5' => return Ok(Kind::Dir),
        b'1' => "a hard link",
        b'2' => "a symbolic link",
        b'3' => "a character device",
        b'4' => "a block device",
        b'6' => "a FIFO",
        b'7' => "a contiguous file",
        b'x' | b'g' => "a pax extended header",
        b'L' | b'K' => "a GNU long name",
        b'S' => "a GNU sparse file",
        _ => "an unknown entry type",
    };
    Err(refused(entry, format!("{what} is refused")))
}

fn header(block: &[u8]) -> Result<Header, ImportError> {
    let stored = octal("header", &block[148..156], "the checksum")?;
    let sum: u64 = block
        .iter()
        .enumerate()
        .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
        .sum();
    if sum != stored {
        return Err(refused("header", "checksum mismatch"));
    }
    let gnu = match &block[257..265] {
        b"ustar\x0000" => false,
        b"ustar  \x00" => true,
        _ => return Err(refused("header", "not a ustar header")),
    };
    let name = text("header", &block[..100], "name")?;
    let prefix = if gnu {
        &[][..]
    } else {
        text("header", &block[345..500], "prefix")?
    };
    let joined = if prefix.is_empty() {
        name.to_vec()
    } else {
        [prefix, b"/", name].concat()
    };
    let path = String::from_utf8(joined).map_err(|_| refused("header", "a non-UTF-8 name"))?;
    let kind = kind(&path, block[156])?;
    if !text(&path, &block[157..257], "link name")?.is_empty() {
        return Err(refused(&path, "a link name on a file or directory"));
    }
    let size = octal(&path, &block[124..136], "the size")?;
    let path = match kind {
        Kind::Dir if size != 0 => return Err(refused(&path, "a directory with a size")),
        Kind::Dir => path.strip_suffix('/').unwrap_or(&path).to_owned(),
        Kind::File => path,
    };
    if !plain_path(&path) {
        return Err(refused(&path, "not a plain relative path"));
    }
    Ok(Header { path, kind, size })
}

fn zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| *b == 0)
}

/// Every entry, by full path, with the files' bytes.
fn entries(stream: &[u8], cap: u64) -> Result<BTreeMap<String, Option<Vec<u8>>>, ImportError> {
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    let mut at = 0usize;
    loop {
        let block = stream
            .get(at..at + BLOCK)
            .ok_or_else(|| refused("archive", "truncated before the end-of-archive blocks"))?;
        if zero(block) {
            let second = stream.get(at + BLOCK..at + 2 * BLOCK);
            if !second.is_some_and(zero) || !zero(&stream[at..]) {
                return Err(refused("archive", "not two zero blocks and zero padding"));
            }
            return Ok(out);
        }
        let h = header(block)?;
        at += BLOCK;
        if out.len() as u64 >= MAX_ENTRIES {
            return Err(refused("archive", format!("entries past {MAX_ENTRIES}")));
        }
        let data = match h.kind {
            Kind::Dir => None,
            Kind::File => {
                if h.size > MAX_THEME_FILE_BYTES {
                    return Err(refused(
                        &h.path,
                        format!("{} bytes is past {MAX_THEME_FILE_BYTES}", h.size),
                    ));
                }
                total += h.size;
                if total > cap {
                    return Err(refused(&h.path, format!("{total} bytes is past {cap}")));
                }
                let size = usize::try_from(h.size).map_err(|_| refused(&h.path, "size"))?;
                let padded = size.div_ceil(BLOCK) * BLOCK;
                let bytes = stream
                    .get(at..at + padded)
                    .ok_or_else(|| refused(&h.path, "data runs past the end"))?;
                at += padded;
                Some(bytes[..size].to_vec())
            }
        };
        if out.insert(h.path.clone(), data).is_some() {
            return Err(refused(&h.path, "a second entry with this path"));
        }
    }
}

/// The archive's files under its one top-level directory, keyed by their
/// path below it.
///
/// # Errors
/// [`ImportError::Archive`] for every refusal in the module docs.
pub fn read(stream: &[u8], cap: u64) -> Result<Tree, ImportError> {
    let all = entries(stream, cap)?;
    let mut tops = all.keys().map(|p| p.split('/').next().unwrap_or_default());
    let top = tops.next().unwrap_or_default().to_owned();
    if tops.any(|t| t != top) {
        return Err(refused("archive", "more than one top-level entry"));
    }
    for path in all.keys() {
        let mut parent = path.as_str();
        while let Some((up, _)) = parent.rsplit_once('/') {
            if matches!(all.get(up), Some(Some(_))) {
                return Err(refused(path, format!("{up} is a file")));
            }
            parent = up;
        }
    }
    let mut tree = Tree::new();
    for (path, data) in all {
        if let Some(bytes) = data {
            let below = path
                .strip_prefix(&format!("{top}/"))
                .ok_or_else(|| refused(&path, "a file at the top level"))?;
            tree.insert(below.to_owned(), bytes);
        }
    }
    if tree.is_empty() {
        return Err(refused("archive", "no files"));
    }
    Ok(tree)
}
