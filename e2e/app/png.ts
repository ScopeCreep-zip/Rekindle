// Plan E7.14 (the shaped-window spec on SK0): read and write the driver's
// screenshots as RGBA pixels, so a window's outline (its alpha) can be
// measured and laid over a backdrop. PNG as the W3C PNG specification
// (Third Edition) defines it: the signature, IHDR (§11.2.1), IDAT's zlib
// stream (§10), and the five scanline filters (§9.2, Paeth §9.4). Only
// what the drivers write is read: 8-bit truecolour with alpha (colour type
// 6), not interlaced; anything else is refused, not guessed.

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

/** The RGBA pixels of the PNG `png`. */
export function decodePng(png: Buffer): Rgba {
  if (!png.subarray(0, 8).equals(SIGNATURE)) throw new Error("not a PNG");
  let width = 0;
  let height = 0;
  const idat: Buffer[] = [];
  for (let at = 8; at < png.length; ) {
    const length = png.readUInt32BE(at);
    const type = png.toString("latin1", at + 4, at + 8);
    const body = png.subarray(at + 8, at + 8 + length);
    if (type === "IHDR") {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const [depth, colour, , , interlace] = body.subarray(8, 13);
      if (depth !== 8 || colour !== 6 || interlace !== 0) {
        throw new Error(`PNG is depth ${depth}, colour type ${colour}, interlace ${interlace}: only 8-bit RGBA, not interlaced, is read`);
      }
    } else if (type === "IDAT") {
      idat.push(body);
    } else if (type === "IEND") {
      break;
    }
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * 4;
  const data = new Uint8Array(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= 4 ? data[y * stride + x - 4] : 0;
      const b = y > 0 ? data[(y - 1) * stride + x] : 0;
      const c = x >= 4 && y > 0 ? data[(y - 1) * stride + x - 4] : 0;
      const predictor = filter === 0 ? 0 : filter === 1 ? a : filter === 2 ? b : filter === 3 ? (a + b) >> 1 : filter === 4 ? paeth(a, b, c) : -1;
      if (predictor < 0) throw new Error(`PNG row ${y} has filter ${filter}`);
      data[y * stride + x] = (line[x] + predictor) & 0xff;
    }
  }
  return { width, height, data };
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
