// Plan E7.14 S5: what each built-in skin declares, read from its package
// (crates/rekindle-skin/builtin/<id>.rkskin), so the skins spec can check the
// real app shows it. A package is a ZIP archive (rekindle-skin `package`)
// holding manifest.json and one DTCG tokens file per variant; this reads
// only those JSON members, through the central directory (PKWARE
// APPNOTE.TXT 6.3.10 §4.3.12, §4.3.16, §4.3.7), stored or deflated (§4.4.5
// methods 0 and 8).

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { crc32, inflateRawSync } from "node:zlib";

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

/**
 * `entries` as a ZIP archive, every member stored (APPNOTE §4.4.5 method 0):
 * a local header and data per member, then the central directory and its
 * end record (§4.3.7, §4.3.12, §4.3.16), as `members` reads them.
 */
function writeZip(entries: Map<string, Buffer>): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(LOCAL, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    parts.push(local, nameBytes, data);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(CENTRAL, 0);
    dir.writeUInt16LE(20, 4); // version made by
    dir.writeUInt16LE(20, 6); // version needed
    dir.writeUInt32LE(crc, 16);
    dir.writeUInt32LE(data.length, 20);
    dir.writeUInt32LE(data.length, 24);
    dir.writeUInt16LE(nameBytes.length, 28);
    dir.writeUInt32LE(offset, 42);
    central.push(dir, nameBytes);
    offset += 30 + nameBytes.length + data.length;
  }
  const dirBytes = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(EOCD, 0);
  end.writeUInt16LE(entries.size, 8);
  end.writeUInt16LE(entries.size, 10);
  end.writeUInt32LE(dirBytes.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, dirBytes, end]);
}

/**
 * A package derived from the built-in `file` for a spec to import through
 * Settings → Import (the real library path): its manifest, renamed `name`
 * under the id `id`, changed by `edit`; every other member as shipped.
 */
export function derivedPack(file: string, id: string, name: string, edit: (manifest: Record<string, unknown>) => void): Buffer {
  const entry = `${file}.rkskin`;
  const zip = members(readFileSync(path.join(BUILTIN_DIR, entry)), entry);
  const bytes = zip.get("manifest.json");
  if (!bytes) throw new Error(`${entry}: no manifest.json`);
  const manifest = JSON.parse(bytes.toString("utf8")) as Record<string, unknown>;
  manifest.id = id;
  manifest.name = name;
  edit(manifest);
  zip.set("manifest.json", Buffer.from(JSON.stringify(manifest), "utf8"));
  return writeZip(zip);
}

/**
 * Symbiosis with its Friends row's Add Friend button pressing Show Voice
 * (plan E7.2 A). No faithful pack lays a Show Voice button in its main or
 * chat window (Xfire's ShowVoiceChatWindow lives in Symbiosis's in-game
 * overlay only, which returns with G15), so the specs import this through
 * Settings → Import, the real library path, and press that node.
 */
export const SHOW_VOICE_PACK = { name: "Show Voice Test", node: "FriendAddButton" };

export function showVoicePack(): Buffer {
  return derivedPack("symbiosis", "show-voice-test", SHOW_VOICE_PACK.name, (m) => {
    const node = findNode((m.windows as Record<string, { tree: unknown }>).main.tree, SHOW_VOICE_PACK.node);
    if (!node) throw new Error(`symbiosis has no ${SHOW_VOICE_PACK.node}`);
    node.triggers = [{ on: { kind: "click" }, run: { kind: "showVoiceWindow" } }];
  });
}

/**
 * Symbiosis with a Search on its Friends row (FriendAddButton) and its Servers
 * row (RefreshServer), pressing `searchFor` (plan E7.2 G11). Symbiosis lays a
 * Search only on its Files (SearchFiles, Downloads' info) and Videos rows, and
 * no built-in pack lays one on Friends or Servers, so the spec imports this
 * through Settings → Import and presses those nodes; Files uses Symbiosis's
 * own SearchFiles.
 */
export const SEARCH_PACK = { name: "Search Test", friends: "FriendAddButton", servers: "RefreshServer", files: "SearchFiles" };

export function searchPack(): Buffer {
  return derivedPack("symbiosis", "search-test", SEARCH_PACK.name, (m) => {
    const tree = (m.windows as Record<string, { tree: unknown }>).main.tree;
    for (const id of [SEARCH_PACK.friends, SEARCH_PACK.servers]) {
      const node = findNode(tree, id);
      if (!node) throw new Error(`symbiosis has no ${id}`);
      node.triggers = [{ on: { kind: "click" }, run: { kind: "searchFor" } }];
    }
  });
}

/** The node `id` in `tree` (a manifest window's `tree`), anywhere under it. */
export function findNode(tree: unknown, id: string): Record<string, unknown> | undefined {
  if (Array.isArray(tree)) {
    for (const n of tree) {
      const found = findNode(n, id);
      if (found) return found;
    }
  } else if (tree !== null && typeof tree === "object") {
    const n = tree as Record<string, unknown>;
    if (n.id === id) return n;
    for (const v of Object.values(n)) {
      const found = findNode(v, id);
      if (found) return found;
    }
  }
  return undefined;
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

/**
 * A pack's name as the Host serves it (plan BR, rekindle-skin
 * skin_tree/brand.rs `name`): without the word "Xfire" (or "Xfire's"), any
 * case, and "Xfire" alone is "Classic". Settings lists skins by this name.
 */
export function servedName(name: string): string {
  const plain = name
    .split(/\s+/)
    .filter((w) => w !== "")
    .filter((w) => {
      const stem = w.replace(/[^A-Za-z0-9]+$/, "");
      return (stem.endsWith("'s") ? stem.slice(0, -2) : stem).toLowerCase() !== "xfire";
    })
    .join(" ");
  return plain === "" ? "Classic" : plain;
}

/** The conforming alternate the Host serves after Symbiosis (plan SK14): Symbiosis re-laid at serve time, no file of its own. */
export const ACCESSIBLE_VARIANT = { file: "symbiosis-access", name: "Symbiosis (Accessible)", after: "symbiosis" };

/** Every built-in skin the app ships, in its order, as its manifest and tokens declare it, by its served name. */
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
    const file = entry.replace(/\.rkskin$/, "");
    packs.push({ file, name: servedName(manifest.name), variants });
    // Its tokens are Symbiosis's; only the layout is re-laid.
    if (file === ACCESSIBLE_VARIANT.after) packs.push({ file: ACCESSIBLE_VARIANT.file, name: ACCESSIBLE_VARIANT.name, variants });
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
