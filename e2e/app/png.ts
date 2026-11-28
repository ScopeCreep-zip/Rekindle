// Plan E7.14 (the shaped-window spec on SK0): read and write the driver's
// screenshots as RGBA pixels, so a window's outline (its alpha) can be
// measured and laid over a backdrop. PNG as the W3C PNG specification
// (Third Edition) defines it: the signature, IHDR (§11.2.1), IDAT's zlib
// stream (§10), and the five scanline filters (§9.2, Paeth §9.4). What the
// drivers write is read: 8-bit, not interlaced, in any colour type (§11.2.1
// table 11.1): truecolour with alpha (6, macOS), truecolour (2,
// WebKitWebDriver on Linux), greyscale (0), greyscale with alpha (4) and
// palette (3, PLTE and tRNS §11.2.2, §11.3.2.1). A type with no alpha is
// opaque except where tRNS names a transparent colour, so a window drawn
// opaque where it should be clear fails a shape check with numbers. Other
// depths and interlacing are refused, not guessed.

import { deflateSync, inflateSync } from "node:zlib";

export interface Rgba {
  width: number;
  height: number;
  /** Row-major, 4 bytes a pixel. */
  data: Uint8Array;
}

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** Bytes a pixel for each 8-bit colour type. */
const CHANNELS: Record<number, number> = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 };

/** The RGBA pixels of the PNG `png`. */
export function decodePng(png: Buffer): Rgba {
  if (!png.subarray(0, 8).equals(SIGNATURE)) throw new Error("not a PNG");
  let width = 0;
  let height = 0;
  let colour = 6;
  let palette: Buffer | null = null;
  let trns: Buffer | null = null;
  const idat: Buffer[] = [];
  for (let at = 8; at < png.length; ) {
    const length = png.readUInt32BE(at);
    const type = png.toString("latin1", at + 4, at + 8);
    const body = png.subarray(at + 8, at + 8 + length);
    if (type === "IHDR") {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const [depth, type, , , interlace] = body.subarray(8, 13);
      if (depth !== 8 || !(type in CHANNELS) || interlace !== 0) {
        throw new Error(`PNG is depth ${depth}, colour type ${type}, interlace ${interlace}: only 8-bit, not interlaced, is read`);
      }
      colour = type;
    } else if (type === "PLTE") {
      palette = body;
    } else if (type === "tRNS") {
      trns = body;
    } else if (type === "IDAT") {
      idat.push(body);
    } else if (type === "IEND") {
      break;
    }
    at += 12 + length;
  }
  if (colour === 3 && !palette) throw new Error("PNG colour type 3 has no PLTE");
  const raw = inflateSync(Buffer.concat(idat));
  const bpp = CHANNELS[colour];
  const stride = width * bpp;
  const px = new Uint8Array(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? px[y * stride + x - bpp] : 0;
      const b = y > 0 ? px[(y - 1) * stride + x] : 0;
      const c = x >= bpp && y > 0 ? px[(y - 1) * stride + x - bpp] : 0;
      const predictor = filter === 0 ? 0 : filter === 1 ? a : filter === 2 ? b : filter === 3 ? (a + b) >> 1 : filter === 4 ? paeth(a, b, c) : -1;
      if (predictor < 0) throw new Error(`PNG row ${y} has filter ${filter}`);
      px[y * stride + x] = (line[x] + predictor) & 0xff;
    }
  }
  if (colour === 6) return { width, height, data: px };
  return { width, height, data: toRgba(px, width * height, colour, palette, trns) };
}

/** Unfiltered 8-bit pixels of `colour` type as RGBA (§11.2.2, §11.3.2.1:
 *  tRNS gives a palette entry's alpha, or the one transparent grey or RGB). */
function toRgba(px: Uint8Array, count: number, colour: number, palette: Buffer | null, trns: Buffer | null): Uint8Array {
  const out = new Uint8Array(count * 4);
  const clearGrey = colour === 0 && trns && trns.length >= 2 ? trns.readUInt16BE(0) : -1;
  const clearRgb = colour === 2 && trns && trns.length >= 6 ? [trns.readUInt16BE(0), trns.readUInt16BE(2), trns.readUInt16BE(4)] : null;
  for (let i = 0; i < count; i++) {
    let r: number, g: number, b: number, a: number;
    if (colour === 0) {
      r = g = b = px[i];
      a = px[i] === clearGrey ? 0 : 255;
    } else if (colour === 4) {
      r = g = b = px[i * 2];
      a = px[i * 2 + 1];
    } else if (colour === 2) {
      [r, g, b] = [px[i * 3], px[i * 3 + 1], px[i * 3 + 2]];
      a = clearRgb && r === clearRgb[0] && g === clearRgb[1] && b === clearRgb[2] ? 0 : 255;
    } else {
      const k = px[i];
      if (!palette || k * 3 + 2 >= palette.length) throw new Error(`PNG palette index ${k} outside PLTE`);
      [r, g, b] = [palette[k * 3], palette[k * 3 + 1], palette[k * 3 + 2]];
      a = trns && k < trns.length ? trns[k] : 255;
    }
    out.set([r, g, b, a], i * 4);
  }
  return out;
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(bytes: Buffer): number {
  let c = 0xffffffff;
  for (const byte of bytes) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, body: Buffer): Buffer {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, "latin1");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])), 0);
  return Buffer.concat([head, body, crc]);
}

/** `image` as a PNG (8-bit RGBA, filter 0 on every row). */
export function encodePng(image: Rgba): Buffer {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(image.width, 0);
  ihdr.writeUInt32BE(image.height, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  const stride = image.width * 4;
  const raw = Buffer.alloc((stride + 1) * image.height);
  for (let y = 0; y < image.height; y++) raw.set(image.data.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  return Buffer.concat([SIGNATURE, chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

/** `image` laid over an opaque `backdrop` colour ([r, g, b]), as the screen composites it. */
export function over(image: Rgba, backdrop: [number, number, number]): Rgba {
  const data = new Uint8Array(image.data.length);
  for (let i = 0; i < data.length; i += 4) {
    const alpha = image.data[i + 3] / 255;
    for (let k = 0; k < 3; k++) data[i + k] = Math.round(image.data[i + k] * alpha + backdrop[k] * (1 - alpha));
    data[i + 3] = 255;
  }
  return { width: image.width, height: image.height, data };
}
