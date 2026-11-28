//! A bounded reader for the two Xfire sound-pack archives: the end record
//! and central directory are authoritative, only stored (0) and deflate
//! (8) entries are read, deflate is inflated with a limit of the entry's
//! declared size, and every CRC-32 is checked. Entry names are flat (no
//! directory part); an encrypted entry, a directory or a name seen twice
//! refuses the archive.

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};

/// The largest archive read.
const MAX_ARCHIVE_BYTES: usize = 8 * 1024 * 1024;
/// The most entries.
const MAX_ENTRIES: usize = 64;
/// The largest entry, uncompressed.
const MAX_ENTRY_BYTES: usize = 4 * 1024 * 1024;
/// An entry's uncompressed bytes per compressed byte.
const MAX_RATIO: usize = 100;
const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const FLAG_ENCRYPTED: u16 = 1;

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    let s = b.get(at..at + 2).context("truncated zip record")?;
    Ok(u16::from_le_bytes(s.try_into()?))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).context("truncated zip record")?;
    Ok(u32::from_le_bytes(s.try_into()?))
}

/// The end of central directory record's offset (no archive comment
/// beyond 64 KiB, as APPNOTE bounds it).
fn eocd(b: &[u8]) -> Result<usize> {
    let lowest = b.len().saturating_sub(22 + 65_535);
    (lowest..=b.len().saturating_sub(22))
        .rev()
        .find(|&at| u32_at(b, at).is_ok_and(|s| s == EOCD_SIG))
        .context("no end of central directory record")
}

/// Every entry of the archive `bytes`, by name.
///
/// # Errors
/// A malformed or out-of-bounds archive.
pub fn unpack(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(bytes.len() <= MAX_ARCHIVE_BYTES, "{} bytes", bytes.len());
    let end = eocd(bytes)?;
    let count = usize::from(u16_at(bytes, end + 10)?);
    ensure!(count <= MAX_ENTRIES, "{count} entries");
    let mut at = u32_at(bytes, end + 16)? as usize;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        ensure!(u32_at(bytes, at)? == CENTRAL_SIG, "bad central header");
        let flags = u16_at(bytes, at + 8)?;
        let method = u16_at(bytes, at + 10)?;
        let crc = u32_at(bytes, at + 16)?;
        let packed = u32_at(bytes, at + 20)? as usize;
        let size = u32_at(bytes, at + 24)? as usize;
        let name_len = usize::from(u16_at(bytes, at + 28)?);
        let skip = usize::from(u16_at(bytes, at + 30)?) + usize::from(u16_at(bytes, at + 32)?);
        let local = u32_at(bytes, at + 42)? as usize;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .context("truncated name")?;
        let name = std::str::from_utf8(name)
            .context("non-UTF-8 name")?
            .to_owned();
        at += 46 + name_len + skip;
        let plain = !name.is_empty() && !name.contains(['/', '\\']) && name != "..";
        ensure!(plain, "entry {name:?} is not a flat file name");
        ensure!(flags & FLAG_ENCRYPTED == 0, "{name}: encrypted");
        ensure!(size <= MAX_ENTRY_BYTES, "{name}: {size} bytes");
        ensure!(
            size <= packed.max(1) * MAX_RATIO,
            "{name}: ratio past {MAX_RATIO}:1"
        );
        ensure!(
            u32_at(bytes, local)? == LOCAL_SIG,
            "{name}: bad local header"
        );
        let data_at = local
            + 30
            + usize::from(u16_at(bytes, local + 26)?)
            + usize::from(u16_at(bytes, local + 28)?);
        let data = data_at
            .checked_add(packed)
            .and_then(|e| bytes.get(data_at..e))
            .with_context(|| format!("{name}: data past the archive"))?;
        let data = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
                .map_err(|e| anyhow::anyhow!("{name}: inflate: {e:?}"))?,
            m => bail!("{name}: method {m}"),
        };
        ensure!(
            data.len() == size,
            "{name}: {} bytes, {size} declared",
            data.len()
        );
        ensure!(crc32fast::hash(&data) == crc, "{name}: CRC-32 mismatch");
        ensure!(out.insert(name.clone(), data).is_none(), "{name} twice");
    }
    Ok(out)
}
