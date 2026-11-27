// The silhouette a built-in skin cuts a window to, computed by the shaper's
// own code (`cargo xtask skin-silhouette`: `rekindle_skin::serve::frames` and
// `rekindle_skin::shape::silhouette` at the window's size and scale), and
// where a window's screenshot disagrees with it: drawn outside it, or
// see-through inside it. Two independent computations of one layout (the
// Rust shaper, and what the webview's CSS painted), so a CSS-side gap or
// misplacement shows; a serve-side frame error both share does not (the
// skin goldens and the specs' named checks cover that).

import { execFileSync } from "node:child_process";
import { REPO_ROOT } from "../walk/launch.ts";
import type { Rgba } from "./png.ts";

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Cut {
  alphaOpaque: number;
  rects: Box[] | null;
  /** The layout's content rectangle (SK2: the views node's box), logical px; null without a frame. */
  content: Box | null;
}

/** The silhouette `pack`'s `frame` cuts a `width × height` window to at `scale` per mille. */
export function silhouette(pack: string, frame: "main" | "chat", width: number, height: number, scale: number): Cut {
  const out = execFileSync(
    "cargo",
    ["run", "-q", "-p", "xtask", "--", "skin-silhouette", "--pack", pack, "--window", frame, "--width", String(width), "--height", String(height), "--scale", String(scale)],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  return JSON.parse(out) as Cut;
}

/**
 * Pixels whose alpha disagrees with `cut`: drawn outside it, or see-through
 * inside it (the window whole when it lays no frame). `columns`, when given,
 * limits the count to those device-pixel columns ([from, to), e.g. a window
 * edge), with up to `log` mismatches named by their coordinates.
 */
export function disagreements(
  img: Rgba,
  cut: Cut,
  columns?: [number, number][],
  log = 6,
  rows?: [number, number][],
): { outsideDrawn: number; insideClear: number; first: string[] } {
  const inside = new Uint8Array(img.width * img.height);
  for (const r of cut.rects ?? [{ x: 0, y: 0, width: img.width, height: img.height }]) {
    for (let y = r.y; y < Math.min(r.y + r.height, img.height); y++) inside.fill(1, y * img.width + r.x, y * img.width + Math.min(r.x + r.width, img.width));
  }
  // A pixel counts when it lies in any listed band (columns or rows); with
  // no bands at all, every pixel counts.
  const inBand = (v: number, bands: [number, number][]): boolean => bands.some(([from, to]) => v >= from && v < to);
  const counted = (x: number, y: number): boolean =>
    (!columns && !rows) || (columns !== undefined && inBand(x, columns)) || (rows !== undefined && inBand(y, rows));
  let outsideDrawn = 0;
  let insideClear = 0;
  const first: string[] = [];
  for (let i = 0; i < inside.length; i++) {
    const x = i % img.width;
    if (!counted(x, Math.floor(i / img.width))) continue;
    const alpha = img.data[i * 4 + 3];
    const drawn = alpha >= cut.alphaOpaque;
    if (drawn === (inside[i] === 1)) continue;
    if (drawn) outsideDrawn++;
    else insideClear++;
    if (first.length < log) first.push(`(${x},${Math.floor(i / img.width)}) alpha ${alpha} ${drawn ? "drawn outside" : "clear inside"}`);
  }
  return { outsideDrawn, insideClear, first };
}
