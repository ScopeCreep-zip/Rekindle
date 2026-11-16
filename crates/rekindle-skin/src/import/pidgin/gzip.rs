//! One gzip member (RFC 1952), inflated in memory with the deflate already
//! in the tree (`miniz_oxide`, as the Winamp importer). Strict: the magic,
//! CM 8 and no reserved flag bit; `FEXTRA`, `FNAME` and `FCOMMENT` are
//! skipped, `FHCRC` is verified. The trailer's `ISIZE` is checked against
//! the cap before anything is allocated, the output buffer is exactly
//! `ISIZE` long (a stream that would write past it is refused, so a bomb
//! with a lying `ISIZE` stops at it), the deflate stream must end exactly
//! at the trailer (no second member, no trailing bytes), and the CRC-32
//! must match.

use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

use super::super::winamp::archive::refused;
use super::super::ImportError;

const ID: [u8; 3] = [0x1f, 0x8b, 8];
const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;
const RESERVED: u8 = 0xe0;
/// ID1 ID2 CM FLG MTIME(4) XFL OS.
const HEADER: usize = 10;
/// CRC32 ISIZE.
const TRAILER: usize = 8;

fn bad(detail: impl Into<String>) -> ImportError {
    refused("gzip", detail)
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// The end of a NUL-terminated header field starting at `at`.
fn past_nul(bytes: &[u8], at: usize, field: &str) -> Result<usize, ImportError> {
    bytes
        .get(at..)
        .and_then(|s| s.iter().position(|b| *b == 0))
        .map(|n| at + n + 1)
        .ok_or_else(|| bad(format!("{field} runs past the end")))
}

/// Where the deflate stream starts.
fn header(bytes: &[u8]) -> Result<usize, ImportError> {
    if bytes.len() < HEADER + TRAILER || bytes[..3] != ID {
        return Err(bad("not a gzip member with deflate (CM 8)"));
    }
    let flags = bytes[3];
    if flags & RESERVED != 0 {
        return Err(bad("a reserved flag bit is set"));
    }
    let mut at = HEADER;
    if flags & FEXTRA != 0 {
        let xlen = bytes
            .get(at..at + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or_else(|| bad("FEXTRA runs past the end"))?;
        at += 2 + xlen;
    }
    if flags & FNAME != 0 {
        at = past_nul(bytes, at, "FNAME")?;
    }
    if flags & FCOMMENT != 0 {
        at = past_nul(bytes, at, "FCOMMENT")?;
    }
    if flags & FHCRC != 0 {
        let stored = bytes
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| bad("FHCRC runs past the end"))?;
        // The low 16 bits of the CRC-32 of every header byte before it.
        if (crc32fast::hash(&bytes[..at]) & 0xffff) as u16 != stored {
            return Err(bad("header CRC-16 mismatch"));
        }
        at += 2;
    }
    if at > bytes.len() - TRAILER {
        return Err(bad("the header runs into the trailer"));
    }
    Ok(at)
}

/// The member's decompressed bytes, at most `cap` of them.
///
/// # Errors
/// [`ImportError::Archive`] for a malformed member, an `ISIZE` past `cap`,
/// a stream longer or shorter than `ISIZE`, trailing bytes or a CRC-32
/// mismatch.
pub fn gunzip(bytes: &[u8], cap: u64) -> Result<Vec<u8>, ImportError> {
    let start = header(bytes)?;
    let end = bytes.len() - TRAILER;
    let crc = le32(&bytes[end..]);
    let size = le32(&bytes[end + 4..]);
    if u64::from(size) > cap {
        return Err(bad(format!("{size} bytes decompressed is past {cap}")));
    }
    let size = usize::try_from(size).map_err(|_| bad("ISIZE does not fit"))?;
    let mut out = vec![0u8; size];
    let mut state = Box::<DecompressorOxide>::default();
    let (status, read, written) = decompress(
        &mut state,
        &bytes[start..end],
        &mut out,
        0,
        inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    match status {
        TINFLStatus::Done => {}
        TINFLStatus::HasMoreOutput => {
            return Err(bad(format!("the stream inflates past ISIZE {size}")));
        }
        other => return Err(bad(format!("deflate: {other:?}"))),
    }
    if read != end - start {
        return Err(bad("bytes after the deflate stream (a second member?)"));
    }
    if written != size {
        return Err(bad(format!("{written} bytes inflated, ISIZE says {size}")));
    }
    if crc32fast::hash(&out) != crc {
        return Err(bad("CRC-32 mismatch"));
    }
    Ok(out)
}
