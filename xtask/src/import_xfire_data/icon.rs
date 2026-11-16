//! One Xfire game icon: an `.ICO` file (ICONDIR, its ICONDIRENTRY table and
//! the images they point at) to one PNG.
//!
//! The largest image is kept (most pixels, then the deepest colour, then
//! the first in the table). A PNG payload is copied as it is, after its
//! signature and IHDR size are checked. A DIB payload (BITMAPINFOHEADER,
//! `BI_RGB`, 1/4/8/24/32 bits a pixel) is converted: the XOR bitmap gives
//! the colour and the 1-bit AND mask the transparency (a set bit is
//! transparent), except a 32-bit image whose own alpha is used, as Windows
//! does, once any of its alpha bytes is non-zero. Every size is bounded
//! before anything is read or allocated.

use anyhow::{bail, ensure, Context, Result};
use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

/// The largest `.ICO` resource read.
pub const MAX_ICO_BYTES: usize = 1024 * 1024;
/// The most images in one `.ICO`.
const MAX_IMAGES: usize = 64;
/// The widest or tallest icon image (the ICO format's own limit).
const MAX_SIDE: u32 = 256;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const BITMAPINFOHEADER: u32 = 40;
const BI_RGB: u32 = 0;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_be_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One image's payload and what ranks it: (pixels, bits a pixel).
struct Candidate<'a> {
    payload: &'a [u8],
    rank: (u32, u16),
}

/// A PNG payload's IHDR width and height, each `1..=MAX_SIDE`.
fn png_size(p: &[u8]) -> Result<(u32, u32)> {
    ensure!(p.starts_with(PNG_SIGNATURE), "not a PNG");
    ensure!(
        p.get(12..16) == Some(b"IHDR".as_slice()),
        "PNG without IHDR first"
    );
    let w = u32_be_at(p, 16).context("short IHDR")?;
    let h = u32_be_at(p, 20).context("short IHDR")?;
    ensure!(
        (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h),
        "PNG {w}x{h} outside 1..={MAX_SIDE}"
    );
    Ok((w, h))
}

/// A DIB payload's header: width, height of the image (half the stored
/// XOR+AND height), bits a pixel and palette length.
struct DibHeader {
    width: u32,
    height: u32,
    bpp: u16,
    colours: usize,
}

fn dib_header(p: &[u8]) -> Result<DibHeader> {
    ensure!(
        u32_at(p, 0) == Some(BITMAPINFOHEADER),
        "not a BITMAPINFOHEADER DIB"
    );
    let width = u32_at(p, 4).context("short DIB header")?;
    let stored = u32_at(p, 8).context("short DIB header")?;
    let bpp = u16_at(p, 14).context("short DIB header")?;
    let compression = u32_at(p, 16).context("short DIB header")?;
    let used = u32_at(p, 32).context("short DIB header")?;
    ensure!(u16_at(p, 12) == Some(1), "DIB planes is not 1");
    ensure!(
        compression == BI_RGB,
        "DIB compression {compression} is not BI_RGB"
    );
    ensure!(matches!(bpp, 1 | 4 | 8 | 24 | 32), "DIB {bpp} bits a pixel");
    ensure!(stored % 2 == 0, "DIB height {stored} is not XOR + AND");
    let height = stored / 2;
    ensure!(
        (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height),
        "DIB {width}x{height} outside 1..={MAX_SIDE}"
    );
    let max_colours = if bpp <= 8 { 1usize << bpp } else { 0 };
    let colours = if used == 0 {
        max_colours
    } else {
        used as usize
    };
    ensure!(
        colours <= max_colours,
        "DIB palette of {colours} past {max_colours}"
    );
    Ok(DibHeader {
        width,
        height,
        bpp,
        colours,
    })
}

fn candidate(payload: &[u8]) -> Result<Candidate<'_>> {
    let rank = if payload.starts_with(PNG_SIGNATURE) {
        let (w, h) = png_size(payload)?;
        // A PNG icon image is 32-bit RGBA (the Vista ICO convention).
        (w * h, 32)
    } else {
        let d = dib_header(payload)?;
        (d.width * d.height, d.bpp)
    };
    Ok(Candidate { payload, rank })
}

/// Bytes a row of `bits`-bit pixels takes, padded to 4 bytes.
fn stride(width: u32, bits: u32) -> usize {
    ((width * bits).div_ceil(32) * 4) as usize
}

/// Pixel `xi` of the XOR row `xor` as `[b, g, r, a]` (`a` is 0 below 32
/// bits a pixel).
fn bgra(hdr: &DibHeader, palette: &[u8], xor: &[u8], xi: usize) -> Result<[u8; 4]> {
    Ok(match hdr.bpp {
        32 => [
            xor[xi * 4],
            xor[xi * 4 + 1],
            xor[xi * 4 + 2],
            xor[xi * 4 + 3],
        ],
        24 => [xor[xi * 3], xor[xi * 3 + 1], xor[xi * 3 + 2], 0],
        bits => {
            let per_byte = 8 / bits as usize;
            let shift = 8 - bits as usize * (xi % per_byte + 1);
            let mask = u8::MAX >> (8 - bits);
            let index = usize::from((xor[xi / per_byte] >> shift) & mask);
            ensure!(index < hdr.colours, "DIB pixel {index} past its palette");
            let at = index * 4;
            [palette[at], palette[at + 1], palette[at + 2], 0]
        }
    })
}

/// The DIB payload as RGBA, rows top-down.
fn dib_to_rgba(payload: &[u8]) -> Result<RgbaImage> {
    let hdr = dib_header(payload)?;
    let palette_at = BITMAPINFOHEADER as usize;
    let xor_at = palette_at + hdr.colours * 4;
    let xor_stride = stride(hdr.width, u32::from(hdr.bpp));
    let and_at = xor_at + xor_stride * hdr.height as usize;
    let and_stride = stride(hdr.width, 1);
    let end = and_at + and_stride * hdr.height as usize;
    ensure!(
        payload.len() >= end,
        "DIB of {} bytes, {end} needed",
        payload.len()
    );
    let palette = &payload[palette_at..xor_at];
    let mut out = RgbaImage::new(hdr.width, hdr.height);
    let mut any_alpha = false;
    for y in 0..hdr.height {
        // DIB rows are stored bottom-up.
        let row = (hdr.height - 1 - y) as usize;
        let xor = &payload[xor_at + row * xor_stride..xor_at + (row + 1) * xor_stride];
        let and = &payload[and_at + row * and_stride..and_at + (row + 1) * and_stride];
        for x in 0..hdr.width {
            let xi = x as usize;
            let [blue, green, red, own] = bgra(&hdr, palette, xor, xi)?;
            let alpha = if hdr.bpp == 32 {
                any_alpha |= own != 0;
                own
            } else if masked(and, xi) {
                0
            } else {
                255
            };
            out.put_pixel(x, y, Rgba([red, green, blue, alpha]));
        }
    }
    if hdr.bpp == 32 && !any_alpha {
        apply_and_mask(&mut out, payload, and_at, and_stride);
    }
    Ok(out)
}

/// Whether pixel `x` of an AND-mask row is set (transparent).
fn masked(and: &[u8], x: usize) -> bool {
    (and[x / 8] >> (7 - x % 8)) & 1 == 1
}

/// Alpha from the AND mask alone (a 32-bit image whose alpha is all zero).
fn apply_and_mask(out: &mut RgbaImage, p: &[u8], and_at: usize, and_stride: usize) {
    let (width, height) = out.dimensions();
    for y in 0..height {
        let row = (height - 1 - y) as usize;
        let and = &p[and_at + row * and_stride..and_at + (row + 1) * and_stride];
        for x in 0..width {
            out.get_pixel_mut(x, y).0[3] = if masked(and, x as usize) { 0 } else { 255 };
        }
    }
}

/// The `.ICO` bytes `ico` (named `name` in errors) as one PNG.
///
/// # Errors
/// A malformed or out-of-bounds ICO, PNG or DIB.
pub fn to_png(name: &str, ico: &[u8]) -> Result<Vec<u8>> {
    convert(ico).with_context(|| name.to_owned())
}

fn convert(ico: &[u8]) -> Result<Vec<u8>> {
    ensure!(ico.len() <= MAX_ICO_BYTES, "ICO of {} bytes", ico.len());
    ensure!(
        u16_at(ico, 0) == Some(0) && u16_at(ico, 2) == Some(1),
        "not an ICONDIR (reserved 0, type 1)"
    );
    let count = usize::from(u16_at(ico, 4).context("short ICONDIR")?);
    ensure!((1..=MAX_IMAGES).contains(&count), "{count} images");
    let mut best: Option<Candidate<'_>> = None;
    for i in 0..count {
        let at = 6 + 16 * i;
        let size = u32_at(ico, at + 8).context("short ICONDIRENTRY")? as usize;
        let offset = u32_at(ico, at + 12).context("short ICONDIRENTRY")? as usize;
        let payload = offset
            .checked_add(size)
            .and_then(|end| ico.get(offset..end))
            .with_context(|| format!("image {i} at {offset}+{size} past the ICO"))?;
        let c = candidate(payload).with_context(|| format!("image {i}"))?;
        if best.as_ref().is_none_or(|b| c.rank > b.rank) {
            best = Some(c);
        }
    }
    let Some(best) = best else {
        bail!("no image");
    };
    if best.payload.starts_with(PNG_SIGNATURE) {
        return Ok(best.payload.to_vec());
    }
    let rgba = dib_to_rgba(best.payload)?;
    encode_png(&DynamicImage::ImageRgba8(rgba)).map_err(|e| anyhow::anyhow!("PNG encode: {e}"))
}

#[cfg(test)]
#[path = "icon_tests.rs"]
mod tests;
