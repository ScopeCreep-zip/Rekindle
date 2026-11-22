// Plan E7.14 S4 (moved from E7.12's e2e/layout/windows.ts): the routed
// windows at the sizes the shell opens them (`src-tauri/src/windows/mod.rs`
// `size()`: default, minimum); the large size is half again the default,
// inside any maximum the shell sets (the buddy list's
// `max_inner_size(400, 900)`). Here the windows are the app's own: a spec
// opens one through the UI, then `checkWindow` sizes it and runs the
// geometry checks.

import { mkdirSync } from "node:fs";
import path from "node:path";
import type { Browser } from "webdriverio";
import { WAIT_MS, sleep, waitUntil } from "../walk/helpers.ts";
import { type Findings, checkGeometry, treeFitIn } from "./geometry.ts";
import type { TreeFit } from "./geometry-page.ts";

export interface Size {
  width: number;
  height: number;
}

export interface WindowCase {
  name: string;
  min: Size;
  def: Size;
  max?: Size;
  /** Containers whose declared gaps must sit on the spacing grid. */
  spacing: string[];
  /** Under a skin's tree, which draws none of the app's bars: the containers
   *  the app itself draws inside the tree, in place of `spacing`. */
  treeSpacing?: string[];
  /** More such containers, rendered only when the window has data. */
  spacingWithData?: string[];
  /** Content panes, rendered when the window has data, that must never scroll sideways. */
  panes?: string[];
  /** Elements this window overlaps on purpose, exempt from the overlap check only (`GeometryOptions.overlapAllow`). */
  overlapAllow?: string[];
}

const SETTINGS_SIZES = { def: { width: 500, height: 550 }, min: { width: 420, height: 450 } };

export const LOGIN: WindowCase = {
  name: "login",
  def: { width: 380, height: 480 },
  min: { width: 340, height: 440 },
  spacing: [".xfire-titlebar"],
};

export const BUDDY_LIST: WindowCase = {
  name: "buddy-list",
  def: { width: 320, height: 650 },
  min: { width: 300, height: 500 },
  max: { width: 400, height: 900 },
  spacing: [".xfire-titlebar", ".status-bar", ".menu-bar"],
  treeSpacing: [".skin-tree-view-content"],
  spacingWithData: [".buddy-group-header"],
  panes: [".buddy-list"],
  // The bell's count badge sits on the glyph's corner by design (lead
  // decision O1, 2026-11-19); its row, clipping and text are still checked.
  overlapAllow: [".notification-badge"],
};

/** The buddy list on its Communities tab: no friend groups, so no group headers. */
export const BUDDY_LIST_COMMUNITIES: WindowCase = {
  ...BUDDY_LIST,
  name: "buddy-list-communities",
  spacingWithData: [],
};

/**
 * Settings → Profile (plan E7.2 game time). Settings opens on this tab
 * (SettingsWindow.tsx `getInitialTab`), so it is also E7.12's "settings"
 * case.
 */
export const SETTINGS_PROFILE: WindowCase = {
  name: "settings-profile",
  ...SETTINGS_SIZES,
  spacing: [".xfire-titlebar", ".form-tabs"],
  panes: [".settings-content"],
};

/** Settings → Privacy (plan E7.1): the quick wipe's fields and buttons beside the switches. */
export const SETTINGS_PRIVACY: WindowCase = {
  name: "settings-privacy",
  ...SETTINGS_SIZES,
  spacing: [".xfire-titlebar", ".form-tabs", ".form-field-row"],
  panes: [".settings-content"],
};

/** Settings → Appearance (plan E7.7.2): the skin list. */
export const SETTINGS_APPEARANCE: WindowCase = {
  name: "settings-appearance",
  ...SETTINGS_SIZES,
  spacing: [".xfire-titlebar", ".form-tabs"],
  spacingWithData: [".skin-list"],
};

export const CHAT_DM: WindowCase = {
  name: "chat-dm",
  def: { width: 480, height: 550 },
  min: { width: 380, height: 400 },
  spacing: [".xfire-titlebar", ".message-input-wrapper", ".message-input-shell"],
  panes: [".chat-message-area"],
};

export const CHAT_GROUP: WindowCase = {
  name: "chat-group",
  def: { width: 480, height: 550 },
  min: { width: 380, height: 400 },
  spacing: [".xfire-titlebar", ".group-chat-header", ".group-chat-members", ".message-input-wrapper"],
  panes: [".chat-message-area"],
};

export const CALL: WindowCase = {
  name: "call",
  def: { width: 420, height: 540 },
  min: { width: 360, height: 420 },
  spacing: [".xfire-titlebar", ".active-call-controls"],
};

export const PROFILE: WindowCase = {
  name: "profile",
  def: { width: 380, height: 500 },
  min: { width: 340, height: 420 },
  spacing: [".xfire-titlebar"],
};

export const COMMUNITY: WindowCase = {
  name: "community",
  def: { width: 900, height: 650 },
  min: { width: 750, height: 500 },
  spacing: [".xfire-titlebar", ".community-sidebar-header"],
  spacingWithData: [".channel-list", ".member-list", ".member-list-header", ".community-channel-header"],
  panes: [".chat-message-area", ".channel-list", ".member-list"],
};

/** Settings → Appearance with a built-in's credits panel open (plan E7.7.3d). */
export const SETTINGS_APPEARANCE_CREDITS: WindowCase = {
  ...SETTINGS_APPEARANCE,
  name: "settings-appearance-credits",
};

/**
 * The skin editor's own window (plan E7.7.5; `windows::open_skin_editor`'s
 * sizes), opened from Settings → Appearance → "Edit a skin…".
 */
export const SKIN_EDITOR: WindowCase = {
  name: "skin-editor",
  def: { width: 640, height: 620 },
  min: { width: 480, height: 480 },
  spacing: [".xfire-titlebar", ".skin-editor-bar"],
  spacingWithData: [".skin-editor-meta", ".skin-region-import", ".skin-pairs"],
};

export function sizes(w: WindowCase): [string, Size][] {
  const large = {
    width: Math.round(w.def.width * 1.5),
    height: Math.round(w.def.height * 1.5),
  };
  if (w.max) {
    large.width = Math.min(large.width, w.max.width);
    large.height = Math.min(large.height, w.max.height);
  }
  return [["min", w.min], ["default", w.def], ["large", large]];
}

/**
 * The minimum the Host serves the window's skin tree (plan SK4
 * `SkinShell.minWidth/minHeight`, the frame's outsets included), or
 * undefined when the window lays no tree. The desktop holds the window to
 * it (`set_min_size`), but WebDriver's Set Window Rect goes below a
 * window's minimum on macOS, so a spec asks for it rather than trusting the
 * OS to refuse a smaller size.
 */
export async function treeMinimum(b: Browser): Promise<Size | undefined> {
  type Got = { width: number; height: number } | { none: true } | { error: string };
  const got = (await b.execute((): Promise<Got> => {
    const frame = document.querySelector(".app-frame");
    if (!frame || !frame.querySelector(".skin-tree")) return Promise.resolve({ none: true });
    const kind = frame.getAttribute("data-skin-window");
    const popup = frame.getAttribute("data-skin-popup");
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } };
    return w.__TAURI_INTERNALS__.invoke("get_skin", { id: null }).then(
      (done): Got => {
        // A Host command answers `{ value, undo }` (ipc/invoke.ts hostInvoke).
        type Shell = { window: string; popup?: { name: string }; minWidth: number; minHeight: number };
        const shell = (done as { value: { shells?: Shell[] } }).value.shells?.find((s) => (popup ? s.popup?.name === popup : s.window === kind && !s.popup));
        return shell ? { width: shell.minWidth, height: shell.minHeight } : { error: `no served shell for ${popup ?? kind ?? "this window"}` };
      },
      (e: unknown): Got => ({ error: `get_skin failed: ${String(e)}` }),
    );
  })) as Got;
  if ("error" in got) throw new Error(got.error);
  return "none" in got ? undefined : got;
}

/**
 * `w`'s sizes for the window `b` is in: a tree-laid window is measured from
 * the minimum its skin serves upward (no axis below it; a size equal to an
 * earlier one is measured once), any other at its kind's sizes.
 */
export async function sizesIn(b: Browser, w: WindowCase): Promise<[string, Size][]> {
  const min = await treeMinimum(b);
  if (!min) return sizes(w);
  const out: [string, Size][] = [];
  for (const [name, s] of sizes(w)) {
    const size = { width: Math.max(s.width, min.width), height: Math.max(s.height, min.height) };
    if (!out.some(([, o]) => o.width === size.width && o.height === size.height)) out.push([name, size]);
  }
  return out;
}

/** The spacing containers of `w` in a window with or without a skin tree (`treeSpacing` under one). */
export function spacingOf(w: WindowCase, tree: boolean, withData: boolean): string[] {
  return [...(tree ? (w.treeSpacing ?? []) : w.spacing), ...(withData ? (w.spacingWithData ?? []) : [])];
}

/**
 * Whether a skin's tree grew with its window across `fits` (one per size):
 * the window's width and height beyond the tree's, and its height beyond
 * the views node's, stay the same, so content fills the window and its
 * foot stays down at every size. A finding per changed gap, else none.
 */
export function treeGrowth(fits: [string, TreeFit][]): string[] {
  const out: string[] = [];
  const first = fits[0];
  if (!first) return out;
  for (const [name, fit] of fits.slice(1)) {
    for (const key of ["gapWidth", "gapHeight", "viewsGapHeight"] as const) {
      if (fit[key] !== first[1][key]) out.push(`${key} ${String(first[1][key])} at ${first[0]}, ${String(fit[key])} at ${name}`);
    }
  }
  return out;
}

async function innerSize(b: Browser): Promise<Size> {
  return b.execute(() => ({ width: window.innerWidth, height: window.innerHeight }));
}

/**
 * Where a checked window is placed: beyond any display (decision D-c). The
 * owner's real pointer otherwise rests over some test window and WebKit
 * applies its `:hover` (the community window's message action bar over the
 * timestamp, S4 full run 2026-11-19), which the driver's synthetic pointer
 * cannot clear. The windows are frameless (`decorations: false`), so macOS
 * keeps an off-screen origin (probe 2026-11-19: −3000,100 / −1000,−1000 /
 * 100,−3000 all kept, `:hover` empty, the screenshot intact). Far enough
 * negative to pass any display arranged left of or above the main one.
 * Wayland compositors place windows themselves, so there it is ignored.
 */
const OFF_SCREEN = { x: -16_000, y: -16_000 };

/**
 * Added to a resize that never lands on a Wayland session: there a native
 * client cannot resize or place its own window (xdg-shell), so a spec that
 * sizes windows runs its app on X11 (`display: "x11"`, launcher.ts).
 */
const WAYLAND_HINT =
  process.platform === "linux" && process.env.WAYLAND_DISPLAY
    ? ' (a Wayland session: a native Wayland client cannot resize its own window, xdg-shell; give this spec\'s config `display: "x11"`)'
    : "";

/**
 * Give the current window the inner (CSS viewport) size `size`, off
 * screen. WebDriver's Set Window Rect sets the outer size, so the frame's
 * difference is read and added back; the resize lands asynchronously, so
 * it waits for the viewport.
 */
export async function sizeWindow(b: Browser, size: Size): Promise<void> {
  await b.setWindowRect(OFF_SCREEN.x, OFF_SCREEN.y, size.width, size.height);
  if (process.platform === "darwin") {
    const rect = await b.getWindowRect();
    if (rect.x !== OFF_SCREEN.x || rect.y !== OFF_SCREEN.y) {
      throw new Error(`the window stayed at ${rect.x},${rect.y}: off-screen placement (D-c) refused, so the real pointer can reach it`);
    }
  }
  let inner = await innerSize(b);
  const dx = size.width - inner.width;
  const dy = size.height - inner.height;
  if (dx !== 0 || dy !== 0) await b.setWindowRect(OFF_SCREEN.x, OFF_SCREEN.y, size.width + dx, size.height + dy);
  await waitUntil(
    async () => {
      inner = await innerSize(b);
      return inner.width === size.width && inner.height === size.height;
    },
    WAIT_MS,
    `window viewport stayed ${JSON.stringify(inner)}, wanted ${JSON.stringify(size)}${WAYLAND_HINT}`,
  );
  // Let layout and any ResizeObserver work land (E7.12 waited 300 ms after
  // its window's IPC settled; a window behind others renders no frames, so
  // a frame callback cannot be the signal).
  await sleep(300);
}

const OUT_DIR = path.join(import.meta.dirname, "out", "layout");

/**
 * The window `b` is in, at each of `w`'s sizes: a screenshot as evidence
 * (e2e/app/out/layout/) and the geometry checks as the pass/fail. Returns
 * findings keyed by size and check; `{}` when every size passes.
 */
export async function checkWindow(b: Browser, w: WindowCase, opts: { withData: boolean; allow?: string[] }): Promise<Findings> {
  mkdirSync(OUT_DIR, { recursive: true });
  const found: Findings = {};
  const fits: [string, TreeFit][] = [];
  for (const [sizeName, size] of await sizesIn(b, w)) {
    await sizeWindow(b, size);
    await b.saveScreenshot(path.join(OUT_DIR, `${w.name}-${sizeName}-${opts.withData ? "data" : "empty"}.png`));
    const fit = await treeFitIn(b, ".app-frame");
    if (fit) fits.push([`${sizeName} ${size.width}x${size.height}`, fit]);
    const findings = await checkGeometry(b, ".app-frame", {
      spacing: spacingOf(w, fit !== null, opts.withData),
      panes: opts.withData ? (w.panes ?? []) : [],
      allow: opts.allow,
      overlapAllow: w.overlapAllow,
    });
    for (const [check, value] of Object.entries(findings)) found[`${sizeName} ${size.width}x${size.height}: ${check}`] = value;
  }
  const growth = treeGrowth(fits);
  if (growth.length > 0) found["tree does not grow with the window"] = growth;
  return found;
}
