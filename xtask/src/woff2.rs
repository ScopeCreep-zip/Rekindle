//! A minimal WOFF2 reader for `check-icon-glyphs`: the table directory, the
//! one Brotli stream, and the two tables the check reads (`cmap` format 12
//! and `post` format 2). Neither is ever transformed (W3C WOFF 2.0 §5.1:
//! only `glyf`, `loca` and `hmtx` have transforms), so their bytes in the
//! decompressed stream are the sfnt tables verbatim.
//!
//! Sources: W3C WOFF File Format 2.0 (Recommendation, 2024-08-08) §4 header,
//! §5.2 table directory and known-tag table, §5.4 compressed stream; OpenType
//! 1.9 `cmap` §"Format 12" and `post` §"Version 2.0".

use std::collections::HashMap;
use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};

/// WOFF 2.0 §5.2 Table 1, "Known Table Tags", indices 0..=62.
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

/// The WOFF2 header is 48 bytes (§4).
const HEADER_LEN: usize = 48;
/// `post` version 2.0 names glyph indices below 258 from the standard
/// Macintosh set; only indices from 258 up name a string in the table.
const MAC_STANDARD_NAMES: u16 = 258;

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| anyhow!("offset overflow"))?;
        let out = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("truncated at {}", self.pos))?;
        self.pos = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// `UIntBase128` (§3.1): at most 5 bytes, no leading zero byte, no overflow.
    fn base128(&mut self) -> Result<u32> {
        let mut acc: u32 = 0;
        for i in 0..5 {
            let byte = self.u8()?;
            if i == 0 && byte == 0x80 {
                bail!("UIntBase128 with a leading zero");
            }
            if acc & 0xFE00_0000 != 0 {
                bail!("UIntBase128 overflow");
            }
            acc = (acc << 7) | u32::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                return Ok(acc);
            }
        }
        bail!("UIntBase128 longer than 5 bytes")
    }
}

fn be_u16(data: &[u8], at: usize) -> Result<u16> {
    Cursor { data, pos: at }.u16()
}

fn be_u32(data: &[u8], at: usize) -> Result<u32> {
    Cursor { data, pos: at }.u32()
}

/// The decompressed sfnt tables of a single-font WOFF2 file, by tag.
pub struct Woff2Tables {
    tables: HashMap<[u8; 4], Vec<u8>>,
}

impl Woff2Tables {
    pub fn parse(file: &[u8]) -> Result<Self> {
        let mut c = Cursor { data: file, pos: 0 };
        if c.take(4)? != b"wOF2" {
            bail!("not a WOFF2 file (signature)");
        }
        let flavor = c.u32()?;
        if flavor == u32::from_be_bytes(*b"ttcf") {
            bail!("font collections are not read");
        }
        let _length = c.u32()?;
        let num_tables = c.u16()?;
        let _reserved = c.u16()?;
        let _total_sfnt = c.u32()?;
        let compressed_len = c.u32()? as usize;
        c.pos = HEADER_LEN;
        let mut dir = Vec::with_capacity(usize::from(num_tables));
        for _ in 0..num_tables {
            let flags = c.u8()?;
            let index = usize::from(flags & 0x3F);
            let tag: [u8; 4] = if index == 0x3F {
                c.take(4)?.try_into().context("tag")?
            } else {
                **KNOWN_TAGS
                    .get(index)
                    .ok_or_else(|| anyhow!("unknown tag index {index}"))?
            };
            let version = flags >> 6;
            let orig = c.base128()?;
            // §5.2: for glyf/loca version 0 is the transform, 3 the null
            // transform; for every other table version 0 is the null one.
            let transformed = if &tag == b"glyf" || &tag == b"loca" {
                version == 0
            } else {
                version != 0
            };
            let stored = if transformed { c.base128()? } else { orig };
            dir.push((tag, stored as usize, transformed));
        }
        let stream = c.take(compressed_len)?;
        let mut raw = Vec::new();
        brotli_decompressor::Decompressor::new(stream, 4096)
            .read_to_end(&mut raw)
            .context("Brotli stream")?;
        let mut tables = HashMap::new();
        let mut at = 0usize;
        for (tag, len, transformed) in dir {
            let end = at
                .checked_add(len)
                .ok_or_else(|| anyhow!("table length overflow"))?;
            let body = raw
                .get(at..end)
                .ok_or_else(|| anyhow!("stream shorter than its directory"))?;
            if !transformed {
                tables.insert(tag, body.to_vec());
            }
            at = end;
        }
        if at != raw.len() {
            bail!(
                "decompressed stream is {} bytes, directory says {at}",
                raw.len()
            );
        }
        Ok(Self { tables })
    }

    fn table(&self, tag: &[u8; 4]) -> Result<&[u8]> {
        self.tables
            .get(tag)
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow!("no `{}` table", String::from_utf8_lossy(tag)))
    }

    /// Codepoint → glyph index from the Unicode full-repertoire subtable
    /// (platform 3 encoding 10, or platform 0 encoding 4/6), format 12.
    pub fn cmap12(&self) -> Result<HashMap<u32, u16>> {
        let t = self.table(b"cmap")?;
        let count = be_u16(t, 2)?;
        for i in 0..usize::from(count) {
            let rec = 4 + i * 8;
            let (platform, encoding) = (be_u16(t, rec)?, be_u16(t, rec + 2)?);
            let off = be_u32(t, rec + 4)? as usize;
            let full =
                (platform == 3 && encoding == 10) || (platform == 0 && matches!(encoding, 4 | 6));
            if !full || be_u16(t, off)? != 12 {
                continue;
            }
            let groups = be_u32(t, off + 12)? as usize;
            let mut map = HashMap::new();
            for g in 0..groups {
                let at = off + 16 + g * 12;
                let (start, end, glyph) = (be_u32(t, at)?, be_u32(t, at + 4)?, be_u32(t, at + 8)?);
                for cp in start..=end {
                    let gid = u16::try_from(glyph + (cp - start)).context("glyph id")?;
                    map.insert(cp, gid);
                }
            }
            return Ok(map);
        }
        bail!("no format 12 Unicode subtable in `cmap`")
    }

    /// Glyph index → name from `post` version 2.0. Standard Macintosh names
    /// (index < 258) are returned as `None`: no icon uses one.
    pub fn post_names(&self) -> Result<Vec<Option<String>>> {
        let t = self.table(b"post")?;
        if be_u32(t, 0)? != 0x0002_0000 {
            bail!("`post` is not version 2.0, so it carries no glyph names");
        }
        let n = usize::from(be_u16(t, 32)?);
        let mut c = Cursor { data: t, pos: 34 };
        let indices = (0..n).map(|_| c.u16()).collect::<Result<Vec<_>>>()?;
        let mut strings = Vec::new();
        while c.pos < t.len() {
            let len = usize::from(c.u8()?);
            strings.push(String::from_utf8(c.take(len)?.to_vec()).context("glyph name")?);
        }
        indices
            .into_iter()
            .map(|i| match i.checked_sub(MAC_STANDARD_NAMES) {
                None => Ok(None),
                Some(k) => strings
                    .get(usize::from(k))
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| anyhow!("glyph name index {i} past the string table")),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "woff2_tests.rs"]
mod tests;
