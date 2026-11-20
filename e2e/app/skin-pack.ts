// Plan E7.14 S5: what each built-in skin declares, read from its package
// (crates/rekindle-skin/builtin/<id>.rkskin), so the skins spec can check the
// real app shows it. A package is a ZIP archive (rekindle-skin `package`)
// holding manifest.json and one DTCG tokens file per variant; this reads
// only those JSON members, through the central directory (PKWARE
// APPNOTE.TXT 6.3.10 §4.3.12, §4.3.16, §4.3.7), stored or deflated (§4.4.5
// methods 0 and 8).

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { inflateRawSync } from "node:zlib";

const BUILTIN_DIR = path.join(import.meta.dirname, "..", "..", "crates", "rekindle-skin", "builtin");

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;
const LOCAL = 0x04034b50;

/** Every member of the archive `zip`, by name, uncompressed. */
function members(zip: Buffer, file: string): Map<string, Buffer> {
  let eocd = -1;
  for (let i = zip.length - 22; i >= 0; i--) {
    if (zip.readUInt32LE(i) === EOCD) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error(`${file}: no end of central directory record`);
  const count = zip.readUInt16LE(eocd + 10);
  let at = zip.readUInt32LE(eocd + 16);
  const out = new Map<string, Buffer>();
  for (let n = 0; n < count; n++) {
    if (zip.readUInt32LE(at) !== CENTRAL) throw new Error(`${file}: bad central directory entry ${n}`);
    const method = zip.readUInt16LE(at + 10);
    const size = zip.readUInt32LE(at + 20);
    const nameLen = zip.readUInt16LE(at + 28);
    const extraLen = zip.readUInt16LE(at + 30);
    const commentLen = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.toString("utf8", at + 46, at + 46 + nameLen);
    at += 46 + nameLen + extraLen + commentLen;
    if (zip.readUInt32LE(local) !== LOCAL) throw new Error(`${file}: bad local header for ${name}`);
    const data = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const raw = zip.subarray(data, data + size);
    if (method === 0) out.set(name, raw);
    else if (method === 8) out.set(name, inflateRawSync(raw));
    else throw new Error(`${file}: ${name} uses compression method ${method}`);
  }
  return out;
}

/** One variant of a skin: its base and every colour token it declares (DTCG path → `#rrggbb[aa]`). */
interface SkinVariant {
  base: "dark" | "light";
  colours: Record<string, string>;
}

export interface SkinPack {
  /** The package file's stem (`xslimfire`). */
  file: string;
  /** The manifest's name, as Settings → Appearance lists it. */
  name: string;
  variants: SkinVariant[];
}

/** The `$type: "color"` leaves of a DTCG tokens document, by dot path. */
function colourLeaves(node: Record<string, unknown>, prefix: string, out: Record<string, string>): void {
  for (const [key, value] of Object.entries(node)) {
    if (value === null || typeof value !== "object") continue;
    const leaf = value as Record<string, unknown>;
    if ("$value" in leaf) {
      if (leaf.$type === "color" && typeof leaf.$value === "string") out[prefix + key] = leaf.$value.toLowerCase();
    } else {
      colourLeaves(leaf, `${prefix + key}.`, out);
    }
  }
}

const BUILTIN_RS = path.join(import.meta.dirname, "..", "..", "crates", "rekindle-skin", "src", "builtin.rs");

/**
 * The packages the app ships as built-in skins, in its order: the
 * `include_bytes!` of `rekindle_skin::builtin::PACKS`. The directory also
 * holds the sound banks (`classic`, `defaults`), which are not skins.
 */
function shippedFiles(): string[] {
  const src = readFileSync(BUILTIN_RS, "utf8");
  const packs = src.slice(src.indexOf("pub const PACKS"));
  const files = [...packs.matchAll(/include_bytes!\("\.\.\/builtin\/([\w-]+\.rkskin)"\)/g)].map((m) => m[1]);
  if (files.length === 0) throw new Error("no packages found in rekindle_skin::builtin::PACKS");
  const present = new Set(readdirSync(BUILTIN_DIR));
  for (const f of files) if (!present.has(f)) throw new Error(`PACKS names ${f}, which is not in ${BUILTIN_DIR}`);
  return files;
}

/** Every built-in skin the app ships, in its order, as its manifest and tokens declare it. */
export function builtinPacks(): SkinPack[] {
  const packs: SkinPack[] = [];
  for (const entry of shippedFiles()) {
    const zip = members(readFileSync(path.join(BUILTIN_DIR, entry)), entry);
    const manifestBytes = zip.get("manifest.json");
    if (!manifestBytes) throw new Error(`${entry}: no manifest.json`);
    const manifest = JSON.parse(manifestBytes.toString("utf8")) as { name: string; variants: { base: "dark" | "light"; tokens: string }[] };
    const variants = manifest.variants.map((v) => {
      const tokens = zip.get(v.tokens);
      if (!tokens) throw new Error(`${entry}: variant tokens ${v.tokens} missing`);
      const colours: Record<string, string> = {};
      colourLeaves(JSON.parse(tokens.toString("utf8")) as Record<string, unknown>, "", colours);
      return { base: v.base, colours };
    });
    packs.push({ file: entry.replace(/\.rkskin$/, ""), name: manifest.name, variants });
  }
  return packs;
}

const TOKENS_CSS = path.join(import.meta.dirname, "..", "..", "src", "styles", "generated", "tokens.css");

/**
 * The registry's default value of each custom property on `base`
 * (src/styles/generated/tokens.css, written by `cargo xtask
 * gen-skin-registry`): the `@theme` block holds the dark defaults, and
 * `:root[data-skin-base="light"]` overrides some of them.
 */
export function baseDefaults(base: "dark" | "light"): Record<string, string> {
  const css = readFileSync(TOKENS_CSS, "utf8");
  const block = (start: string): Record<string, string> => {
    const from = css.indexOf(start);
    if (from < 0) throw new Error(`tokens.css has no ${start} block`);
    const body = css.slice(css.indexOf("{", from) + 1, css.indexOf("}", from));
    const out: Record<string, string> = {};
    for (const m of body.matchAll(/(--[\w-]+):\s*([^;]+);/g)) out[m[1]] = m[2].trim().toLowerCase();
    return out;
  };
  const dark = block("@theme {");
  return base === "dark" ? dark : { ...dark, ...block(':root[data-skin-base="light"] {') };
}
