//! The zip records, read strictly (PKWARE APPNOTE 6.3.10 §4.3): the end of
//! central directory, the central directory headers and the local headers.
//! Nothing here inflates, seeks outside the buffer or trusts one record
//! without its twin: every local header must repeat its central record.

use rekindle_types::dto::skin::{SkinError, SkinRule};

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;
const MAX_COMMENT: usize = 0xFFFF;

/// General purpose flag bits (APPNOTE §4.4.4).
pub(super) const FLAG_ENCRYPTED: u16 = 1;
pub(super) const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
pub(super) const FLAG_UTF8: u16 = 1 << 11;

pub(super) fn malformed(entry: &str, detail: &str) -> SkinError {
    SkinError::new(
        entry,
        SkinRule::MalformedZip {
            detail: detail.to_owned(),
        },
    )
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn usize_of(v: u32) -> usize {
    // A u32 always fits: the crate builds for 32- and 64-bit targets only.
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Where the central directory is and how many records it holds.
pub(super) struct Directory {
    pub offset: usize,
    pub end: usize,
    pub count: usize,
}

/// Finds the end of central directory record: the last signature whose
/// comment length reaches exactly the end of the buffer. A single disk,
/// matching counts, and a central directory that ends where the record
/// begins.
pub(super) fn directory(bytes: &[u8]) -> Result<Directory, SkinError> {
    let pkg = SkinError::PACKAGE;
    let last = bytes
        .len()
        .checked_sub(EOCD_LEN)
        .ok_or_else(|| malformed(pkg, "too short for an end of central directory"))?;
    let first = last.saturating_sub(MAX_COMMENT);
    let at = (first..=last)
        .rev()
        .find(|&at| {
            u32_at(bytes, at) == Some(EOCD_SIG)
                && u16_at(bytes, at + 20).map(usize::from) == Some(bytes.len() - at - EOCD_LEN)
        })
        .ok_or_else(|| malformed(pkg, "no end of central directory"))?;
    let field = |off| u16_at(bytes, at + off).unwrap_or(u16::MAX);
    if field(4) != 0 || field(6) != 0 || field(8) != field(10) {
        return Err(malformed(pkg, "multi-disk archive"));
    }
    let count = usize::from(field(10));
    let size = usize_of(u32_at(bytes, at + 12).unwrap_or(u32::MAX));
    let offset = usize_of(u32_at(bytes, at + 16).unwrap_or(u32::MAX));
    if offset.checked_add(size) != Some(at) {
        return Err(malformed(
            pkg,
            "central directory does not end at its end record",
        ));
    }
    Ok(Directory {
        offset,
        end: at,
        count,
    })
}

/// One central directory header.
pub(super) struct Central<'a> {
    pub name: &'a [u8],
    pub made_by: u16,
    pub flags: u16,
    pub method: u16,
    pub crc: u32,
    pub compressed: u32,
    pub uncompressed: u32,
    pub external_attrs: u32,
    pub local_offset: usize,
}

/// Reads the central header at `at`; returns it and the next header's offset.
pub(super) fn central(bytes: &[u8], at: usize) -> Result<(Central<'_>, usize), SkinError> {
    let bad = || malformed(SkinError::PACKAGE, "truncated central directory header");
    if u32_at(bytes, at) != Some(CENTRAL_SIG) {
        return Err(malformed(
            SkinError::PACKAGE,
            "bad central directory signature",
        ));
    }
    let h16 = |off| u16_at(bytes, at + off).ok_or_else(bad);
    let h32 = |off| u32_at(bytes, at + off).ok_or_else(bad);
    let name_len = usize::from(h16(28)?);
    let extra_len = usize::from(h16(30)?);
    let comment_len = usize::from(h16(32)?);
    let name_start = at + CENTRAL_LEN;
    let name = bytes
        .get(name_start..name_start + name_len)
        .ok_or_else(bad)?;
    let next = name_start + name_len + extra_len + comment_len;
    if next > bytes.len() {
        return Err(bad());
    }
    let header = Central {
        name,
        made_by: h16(4)?,
        flags: h16(8)?,
        method: h16(10)?,
        crc: h32(16)?,
        compressed: h32(20)?,
        uncompressed: h32(24)?,
        external_attrs: h32(38)?,
        local_offset: usize_of(h32(42)?),
    };
    if h16(34)? != 0 {
        return Err(malformed(SkinError::PACKAGE, "multi-disk archive"));
    }
    Ok((header, next))
}

/// Checks the local header against its central record and returns the
/// byte range of the entry's data, which must end before the central
/// directory.
pub(super) fn local_data(
    bytes: &[u8],
    entry: &str,
    c: &Central<'_>,
    directory_start: usize,
) -> Result<std::ops::Range<usize>, SkinError> {
    let at = c.local_offset;
    let bad = || malformed(entry, "local header disagrees with the central directory");
    if u32_at(bytes, at) != Some(LOCAL_SIG) {
        return Err(malformed(entry, "bad local header signature"));
    }
    let h16 = |off| u16_at(bytes, at + off).ok_or_else(bad);
    let h32 = |off| u32_at(bytes, at + off).ok_or_else(bad);
    if h16(6)? != c.flags
        || h16(8)? != c.method
        || h32(14)? != c.crc
        || h32(18)? != c.compressed
        || h32(22)? != c.uncompressed
    {
        return Err(bad());
    }
    let name_len = usize::from(h16(26)?);
    let extra_len = usize::from(h16(28)?);
    let name_start = at + LOCAL_LEN;
    if bytes.get(name_start..name_start + name_len) != Some(c.name) {
        return Err(bad());
    }
    let start = name_start + name_len + extra_len;
    let end = start
        .checked_add(usize_of(c.compressed))
        .filter(|&end| end <= directory_start)
        .ok_or_else(|| malformed(entry, "entry data runs into the central directory"))?;
    Ok(start..end)
}
