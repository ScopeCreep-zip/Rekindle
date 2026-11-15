//! A bounded Windows BMP decoder for the Winamp sheets only (plan E7.7.3:
//! "RLE BMP is decoded only here"; `rekindle-image` has no BMP decoder and
//! gains none). Accepted: a `BITMAPINFOHEADER` (40 bytes) or its V4/V5
//! extensions (108/124), 1-, 4- and 8-bit palettes and 24-/32-bit pixels
//! uncompressed (`BI_RGB`), and 8-bit `BI_RLE8`. Everything else (the OS/2
//! core header, `BI_RLE4`, `BI_BITFIELDS`, embedded JPEG/PNG, 16-bit) is
//! refused. Sides and pixels are capped by the validator's image limits
//! before anything is allocated, and every read is bounds-checked.
//! Format: Microsoft "Bitmap Storage" (BITMAPFILEHEADER, BITMAPINFOHEADER,
//! "Bitmap Compression"). Pixels an RLE8 stream skips stay transparent.

use rekindle_image::{Rgba, RgbaImage};

use super::super::ImportError;
use crate::limits::{MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};

const FILE_HEADER: usize = 14;
const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;

fn bad(file: &str, detail: impl std::fmt::Display) -> ImportError {
    ImportError::Image {
        file: file.to_owned(),
        detail: format!("BMP: {detail}"),
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

struct Header {
    width: u32,
    height: u32,
    top_down: bool,
    bits: u16,
    compression: u32,
    palette: Vec<[u8; 3]>,
    data: usize,
}

fn header(file: &str, bytes: &[u8]) -> Result<Header, ImportError> {
    let trunc = || bad(file, "truncated header");
    if bytes.get(..2) != Some(b"BM") {
        return Err(bad(file, "no BM signature"));
    }
    let data = u32_at(bytes, 10).ok_or_else(trunc)? as usize;
    let size = u32_at(bytes, FILE_HEADER).ok_or_else(trunc)? as usize;
    if !matches!(size, 40 | 108 | 124) {
        return Err(bad(file, format!("header size {size} is refused")));
    }
    let sw = i32_at(bytes, FILE_HEADER + 4).ok_or_else(trunc)?;
    let sh = i32_at(bytes, FILE_HEADER + 8).ok_or_else(trunc)?;
    let bits = u16_at(bytes, FILE_HEADER + 14).ok_or_else(trunc)?;
    let compression = u32_at(bytes, FILE_HEADER + 16).ok_or_else(trunc)?;
    let used = u32_at(bytes, FILE_HEADER + 32).ok_or_else(trunc)?;
    let (width, height) = (sw.unsigned_abs(), sh.unsigned_abs());
    if sw <= 0 || sh == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(bad(
            file,
            format!("{sw}x{sh} is outside 1..={MAX_IMAGE_SIDE}"),
        ));
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(bad(file, "too many pixels"));
    }
    match (bits, compression) {
        (1 | 4 | 8 | 24 | 32, BI_RGB) | (8, BI_RLE8) => {}
        _ => return Err(bad(file, format!("{bits}-bit compression {compression}"))),
    }
    if compression == BI_RLE8 && sh < 0 {
        return Err(bad(file, "a top-down RLE8 bitmap"));
    }
    let mut palette = Vec::new();
    if bits <= 8 {
        let max = 1u32 << bits;
        let n = if used == 0 { max } else { used };
        if n > max {
            return Err(bad(file, format!("{n} palette entries for {bits}-bit")));
        }
        let at = FILE_HEADER + size;
        for i in 0..n as usize {
            let e = bytes.get(at + i * 4..at + i * 4 + 3).ok_or_else(trunc)?;
            palette.push([e[2], e[1], e[0]]);
        }
    }
    Ok(Header {
        width,
        height,
        top_down: sh < 0,
        bits,
        compression,
        palette,
        data,
    })
}

impl Header {
    fn colour(&self, file: &str, index: u8) -> Result<Rgba<u8>, ImportError> {
        let [r, g, b] = *self
            .palette
            .get(usize::from(index))
            .ok_or_else(|| bad(file, format!("palette index {index} out of range")))?;
        Ok(Rgba([r, g, b, 255]))
    }

    /// The image row of stored row `i`.
    fn row(&self, i: u32) -> u32 {
        if self.top_down {
            i
        } else {
            self.height - 1 - i
        }
    }
}

/// Decodes `bytes` into RGBA.
///
/// # Errors
/// [`ImportError::Image`] for a refused or malformed bitmap.
pub fn decode(file: &str, bytes: &[u8]) -> Result<RgbaImage, ImportError> {
    let h = header(file, bytes)?;
    let data = bytes
        .get(h.data..)
        .ok_or_else(|| bad(file, "pixel data offset past the end"))?;
    let mut out = RgbaImage::new(h.width, h.height);
    if h.compression == BI_RLE8 {
        rle8(file, &h, data, &mut out)?;
    } else {
        uncompressed(file, &h, data, &mut out)?;
    }
    Ok(out)
}

fn uncompressed(
    file: &str,
    h: &Header,
    data: &[u8],
    out: &mut RgbaImage,
) -> Result<(), ImportError> {
    let bits = usize::from(h.bits);
    let stride = (h.width as usize * bits).div_ceil(32) * 4;
    for i in 0..h.height {
        let at = i as usize * stride;
        let row = data
            .get(at..at + stride)
            .ok_or_else(|| bad(file, "pixel data truncated"))?;
        let y = h.row(i);
        for x in 0..h.width {
            let xi = x as usize;
            let p = match h.bits {
                24 | 32 => {
                    let o = xi * bits / 8;
                    Rgba([row[o + 2], row[o + 1], row[o], 255])
                }
                _ => {
                    let bit = xi * bits;
                    let byte = row[bit / 8];
                    let shift = 8 - bits - bit % 8;
                    let mask = u8::try_from((1u16 << bits) - 1).unwrap_or(u8::MAX);
                    h.colour(file, (byte >> shift) & mask)?
                }
            };
            out.put_pixel(x, y, p);
        }
    }
    Ok(())
}

fn rle8(file: &str, hdr: &Header, data: &[u8], out: &mut RgbaImage) -> Result<(), ImportError> {
    let (mut x, mut i, mut at) = (0u32, 0u32, 0usize);
    let next = |at: &mut usize| -> Result<u8, ImportError> {
        let v = *data
            .get(*at)
            .ok_or_else(|| bad(file, "RLE8 stream truncated"))?;
        *at += 1;
        Ok(v)
    };
    let mut put = |x: &mut u32, i: u32, index: u8| -> Result<(), ImportError> {
        if *x >= hdr.width || i >= hdr.height {
            return Err(bad(file, "RLE8 run past the bitmap"));
        }
        out.put_pixel(*x, hdr.row(i), hdr.colour(file, index)?);
        *x += 1;
        Ok(())
    };
    loop {
        let (count, value) = (next(&mut at)?, next(&mut at)?);
        if count > 0 {
            for _ in 0..count {
                put(&mut x, i, value)?;
            }
            continue;
        }
        match value {
            0 => {
                x = 0;
                i += 1;
            }
            1 => return Ok(()),
            2 => {
                x += u32::from(next(&mut at)?);
                i += u32::from(next(&mut at)?);
                if x > hdr.width || i > hdr.height {
                    return Err(bad(file, "RLE8 delta past the bitmap"));
                }
            }
            n => {
                for _ in 0..n {
                    let v = next(&mut at)?;
                    put(&mut x, i, v)?;
                }
                if n % 2 == 1 {
                    next(&mut at)?;
                }
            }
        }
        if i > hdr.height {
            return Err(bad(file, "RLE8 rows past the bitmap"));
        }
    }
}
