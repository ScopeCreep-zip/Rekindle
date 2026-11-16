//! A bounded Truevision TGA decoder for the Clist_modern atlases only
//! (plan E7.7.3 item 6: real `.msf` skins draw from TGA atlases;
//! `rekindle-image` has no TGA decoder and gains none, as Winamp's RLE BMP
//! is decoded only by [`super::super::winamp::bmp`]). Accepted: image type
//! 2 (uncompressed true-colour) and 10 (RLE true-colour), 24-bit with no
//! attribute bits or 32-bit with 8 alpha bits, either origin corner. A
//! colour map (or a colour-map spec on a true-colour image), any other
//! image type, depth or attribute count, and interleaving are refused. The
//! image ID is skipped by its declared length; the footer and extension
//! area are never read. Sides and pixels are capped by the validator's
//! image limits before anything is allocated, every read is
//! bounds-checked, and an RLE packet that would run past the last pixel is
//! refused. Format: Truevision "TGA File Format Specification" 2.0, "Data
//! Type 2" and "Data Type 10" (18-byte header, BGR(A) pixels, packet
//! header bit 7 = run, low 7 bits = count − 1).

use rekindle_image::{Rgba, RgbaImage};

use super::super::raster::{refused, u16_at, within_limits};
use super::super::ImportError;

const HEADER: usize = 18;
const TRUE_COLOUR: u8 = 2;
const RLE_TRUE_COLOUR: u8 = 10;
/// Descriptor bit 4: pixels run right to left.
const RIGHT_TO_LEFT: u8 = 0x10;
/// Descriptor bit 5: the first row is the top one.
const TOP_DOWN: u8 = 0x20;
/// Descriptor bits 6-7: interleaving (refused).
const INTERLEAVE: u8 = 0xC0;

fn bad(file: &str, detail: impl std::fmt::Display) -> ImportError {
    refused("TGA", file, detail)
}

struct Header {
    width: u32,
    height: u32,
    rle: bool,
    /// Bytes per pixel, 3 or 4.
    depth: usize,
    descriptor: u8,
    data: usize,
}

fn header(file: &str, bytes: &[u8]) -> Result<Header, ImportError> {
    let h = bytes
        .get(..HEADER)
        .ok_or_else(|| bad(file, "truncated header"))?;
    let (id_len, map_type, kind) = (h[0], h[1], h[2]);
    if map_type != 0 || h[3..8].iter().any(|b| *b != 0) {
        return Err(bad(file, "a colour map is refused"));
    }
    let rle = match kind {
        TRUE_COLOUR => false,
        RLE_TRUE_COLOUR => true,
        k => return Err(bad(file, format!("image type {k} is refused"))),
    };
    let width = u32::from(u16_at(h, 12).unwrap_or_default());
    let height = u32::from(u16_at(h, 14).unwrap_or_default());
    within_limits(width, height).map_err(|why| bad(file, why))?;
    let (bits, descriptor) = (h[16], h[17]);
    let depth = match (bits, descriptor & 0x0F) {
        (24, 0) => 3,
        (32, 8) => 4,
        (b, a) => return Err(bad(file, format!("{b}-bit with {a} attribute bits"))),
    };
    if descriptor & INTERLEAVE != 0 {
        return Err(bad(file, "interleaving is refused"));
    }
    Ok(Header {
        width,
        height,
        rle,
        depth,
        descriptor,
        data: HEADER + usize::from(id_len),
    })
}

impl Header {
    /// Image coordinates of stored pixel `index` (below `width × height`,
    /// which the validator's limits keep within `u32`).
    fn at(&self, index: u32) -> (u32, u32) {
        let (col, row) = (index % self.width, index / self.width);
        let x = if self.descriptor & RIGHT_TO_LEFT == 0 {
            col
        } else {
            self.width - 1 - col
        };
        let y = if self.descriptor & TOP_DOWN == 0 {
            self.height - 1 - row
        } else {
            row
        };
        (x, y)
    }

    fn pixel(&self, bgra: &[u8]) -> Rgba<u8> {
        let alpha = if self.depth == 4 { bgra[3] } else { u8::MAX };
        Rgba([bgra[2], bgra[1], bgra[0], alpha])
    }

    /// Writes `pixels` (BGR(A), `depth` bytes each) from stored index `first`.
    fn put(&self, out: &mut RgbaImage, first: u32, pixels: &[u8]) {
        for (index, bgra) in (first..).zip(pixels.chunks_exact(self.depth)) {
            let (x, y) = self.at(index);
            out.put_pixel(x, y, self.pixel(bgra));
        }
    }
}

/// Decodes `bytes` into RGBA.
///
/// # Errors
/// [`ImportError::Image`] for a refused or malformed TGA.
pub fn decode(file: &str, bytes: &[u8]) -> Result<RgbaImage, ImportError> {
    let hdr = header(file, bytes)?;
    let data = bytes
        .get(hdr.data..)
        .ok_or_else(|| bad(file, "image ID past the end"))?;
    let mut out = RgbaImage::new(hdr.width, hdr.height);
    if hdr.rle {
        rle(file, &hdr, data, &mut out)?;
    } else {
        let len = hdr.width as usize * hdr.height as usize * hdr.depth;
        let pixels = data
            .get(..len)
            .ok_or_else(|| bad(file, "pixel data truncated"))?;
        hdr.put(&mut out, 0, pixels);
    }
    Ok(out)
}

/// Packets in stored order; a packet that would run past the last pixel
/// is refused before anything of it is written.
fn rle(file: &str, hdr: &Header, data: &[u8], out: &mut RgbaImage) -> Result<(), ImportError> {
    let total = hdr.width * hdr.height;
    let (mut done, mut at) = (0u32, 0usize);
    let mut take = |len: usize| -> Result<&[u8], ImportError> {
        let bytes = data
            .get(at..at + len)
            .ok_or_else(|| bad(file, "RLE stream truncated"))?;
        at += len;
        Ok(bytes)
    };
    while done < total {
        let packet = take(1)?[0];
        let count = usize::from(packet & 0x7F) + 1;
        let end = done + u32::from(packet & 0x7F) + 1;
        if end > total {
            return Err(bad(file, "RLE packet past the image"));
        }
        if packet & 0x80 != 0 {
            let run = take(hdr.depth)?.repeat(count);
            hdr.put(out, done, &run);
        } else {
            hdr.put(out, done, take(count * hdr.depth)?);
        }
        done = end;
    }
    Ok(())
}
