// Plan E7.14 on SK0 (2026-11-20): a skinned window is the skin's shape, not
// a skinned box inside a rectangle. For each built-in skin, the buddy list
// and a chat window are screenshotted. The driver's screenshot is the
// webview's own snapshot with its alpha (WKWebView on macOS; WebKitGTK with
// TRANSPARENT_BACKGROUND on Linux), so no screen capture and no permission
// prompt is needed. Each is saved raw and laid over two backdrops, in
// e2e/app/out/shape/<skin>/<window>-{raw,over-grey,over-magenta}.png, to
// compare with the original skin's screenshots.
//
// The window's drawing must be its cut. The silhouette the shaper cuts the
// window to is computed by `cargo xtask skin-silhouette`: the shaper's own
// computation, rekindle_skin::shape::silhouette of the pack's frame, at the
// window's size and the screenshot's scale. The page never receives a
// silhouette (check-no-shape-ipc). Every pixel outside it must be
// see-through (alpha below the shaper's ALPHA_OPAQUE) and every pixel
// inside it drawn (at or above). That catches a skinned box drawn inside a
// rectangle ("an app within an app"), art outside the cut, holes inside
// it, and on Linux a client-side shadow, which GTK draws in the surface's
// margin. It runs for each skin in turn (each is a skin change), at the
// size each window opens at, and, where a client can resize itself (not
// native Wayland), again after a resize.
//
// Owed manual checks, one time each, since a test cannot do them here:
// - macOS: a click on a cut corner (Xfire Classic's rounded top) reaches
//   the window behind, and macOS draws no shadow round the window. A real
//   WindowServer click needs Accessibility (a TCC prompt) and moves the
//   owner's pointer. The shadow is drawn outside the webview's snapshot,
//   and reading NSWindow.hasShadow would need new unsafe code in the
//   driver. base_builder's shadow(false) sets it off.
// - Wayland: a click outside the silhouette reaches the window behind.
//   A client cannot read back the compositor's input region, and
//   synthetic input needs uinput.
// - X11: the server's input and bounding shapes, and real clicks, belong
//   in a nested Xvfb with an XTEST client, never the owner's display. The
//   Linux laptop has neither Xvfb nor xdotool yet, so that check skips,
//   saying so.

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { REPO_ROOT } from "../../../walk/launch.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, newUser, openChat, type User } from "../../journeys.ts";
import { decodePng, encodePng, over, type Rgba } from "../../png.ts";
import { builtinPacks, type SkinPack } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";
import { BUDDY_LIST, sizeWindow } from "../../windows.ts";

const OUT = path.join(import.meta.dirname, "..", "..", "out", "shape");
const PACKS = builtinPacks();
const NATIVE_WAYLAND = process.platform === "linux" && Boolean(process.env.WAYLAND_DISPLAY) && process.env.GDK_BACKEND !== "x11";

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface Cut {
  alphaOpaque: number;
  rects: Box[] | null;
  /** The layout's content rectangle and header region (plan SK0-L1), logical px; null without a frame / header region. */
  content: Box | null;
  header: Box | null;
}

/** Where `a` and `b` differ by more than 1 px on any edge, as text; empty when they agree. */
function offBy(a: Box, b: Box): string {
  const far = Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y), Math.abs(a.width - b.width), Math.abs(a.height - b.height));
  return far <= 1 ? "" : `${JSON.stringify(a)} is not ${JSON.stringify(b)}`;
}

/**
 * The content box of the window's `.app-frame` (its rect less its padding), its profile header's rect, logical px, and
 * how many `.buddy-item` rows show inside the content box (a row's centre inside it, and the element drawn at that
 * centre is the row's own: a collapsed or clipped list shows none).
 */
async function pageBoxes(): Promise<{ content: Box; header: Box | null; rows: number }> {
  return (await browser.execute(() => {
    const root = document.querySelector(".app-frame") as HTMLElement;
    const r = root.getBoundingClientRect();
    const cs = getComputedStyle(root);
    const p = (k: string): number => parseFloat(cs.getPropertyValue(k)) || 0;
    const content = {
      x: r.left + p("padding-left"),
      y: r.top + p("padding-top"),
      width: r.width - p("padding-left") - p("padding-right"),
      height: r.height - p("padding-top") - p("padding-bottom"),
    };
    const bar = root.querySelector(":scope > .identity-bar");
    const h = bar ? bar.getBoundingClientRect() : null;
    const rows = Array.from(root.querySelectorAll(".buddy-item")).filter((row) => {
      const b = row.getBoundingClientRect();
      const cx = b.left + b.width / 2;
      const cy = b.top + b.height / 2;
      const inside = cx >= content.x && cx <= content.x + content.width && cy >= content.y && cy <= content.y + content.height;
      const hit = document.elementFromPoint(cx, cy);
      return b.width > 0 && b.height > 0 && inside && hit !== null && row.contains(hit);
    }).length;
    return { content, header: h ? { x: h.left, y: h.top, width: h.width, height: h.height } : null, rows };
  })) as { content: Box; header: Box | null; rows: number };
}

/** The silhouette `pack`'s `frame` cuts a `width × height` window to at `scale` per mille. */
function silhouette(pack: string, frame: "main" | "chat", width: number, height: number, scale: number): Cut {
  const out = execFileSync(
    "cargo",
    ["run", "-q", "-p", "xtask", "--", "skin-silhouette", "--pack", pack, "--window", frame, "--width", String(width), "--height", String(height), "--scale", String(scale)],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  return JSON.parse(out) as Cut;
}

/** Pixels whose alpha disagrees with `cut`: drawn outside it, or see-through inside it (the window whole when it lays no frame). */
function disagreements(img: Rgba, cut: Cut): { outsideDrawn: number; insideClear: number; first: string[] } {
  const inside = new Uint8Array(img.width * img.height);
  for (const r of cut.rects ?? [{ x: 0, y: 0, width: img.width, height: img.height }]) {
    for (let y = r.y; y < Math.min(r.y + r.height, img.height); y++) inside.fill(1, y * img.width + r.x, y * img.width + Math.min(r.x + r.width, img.width));
  }
  let outsideDrawn = 0;
  let insideClear = 0;
  const first: string[] = [];
  for (let i = 0; i < inside.length; i++) {
    const alpha = img.data[i * 4 + 3];
    const drawn = alpha >= cut.alphaOpaque;
    if (drawn === (inside[i] === 1)) continue;
    if (drawn) outsideDrawn++;
    else insideClear++;
    if (first.length < 6) first.push(`(${i % img.width},${Math.floor(i / img.width)}) alpha ${alpha} ${drawn ? "drawn outside" : "clear inside"}`);
  }
  return { outsideDrawn, insideClear, first };
}

describe("a skinned window is the skin's shape", function () {
  this.timeout(1_800_000);
  const me: User = newUser("shape");
  let peers: Peer[] = [];
  const handles: Record<string, string> = {};

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["PeerOne"]);
    await befriendFleet(browser, me, peers);
    await openChat(browser, peers[0].displayName);
    handles.chat = await browser.getWindowHandle();
    await openSettings(browser, "Appearance");
    await shown(browser, ".skin-row", NET_WAIT_MS);
  });

  after(async () => {
    await stopFleet(peers);
  });

  /** Apply `pack` from Settings → Appearance (its radio, then the intent review). */
  async function apply(pack: SkinPack): Promise<string> {
    await switchTo(browser, SETTINGS);
    await (await byText(browser, ".form-tab", "Appearance")).click();
    await shown(browser, ".skin-row-shown .skin-name", NET_WAIT_MS);
    const inUse = async (): Promise<string> => (await (await browser.$(".skin-row-shown .skin-name")).getText()).trim();
    const shownHref = (): Promise<string> =>
      browser.execute('var l = document.getElementById("rekindle-skin"); return l && l.sheet ? l.getAttribute("href") : "";') as Promise<string>;
    // A changed skin must be a different sheet than the one shown before
    // the change: the previous skin's loaded sheet never passes for it.
    let before = "";
    if ((await inUse()) !== pack.name) {
      before = await shownHref();
      for (const r of await browser.$$(".skin-row")) {
        if ((await (await r.$(".skin-name")).getText()).trim() !== pack.name) continue;
        await (await r.$("input[type=radio]")).click();
        await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
        break;
      }
      await waitUntil(async () => (await inUse()) === pack.name, NET_WAIT_MS, `${pack.name} never shown as the skin in use`);
    }
    let href = "";
    await waitUntil(
      async () => {
        href = await shownHref();
        return href.startsWith("skin://") && href !== before;
      },
      NET_WAIT_MS,
      "Settings never linked the skin's sheet",
    );
    return href;
  }

  /** The window `browser` is in, shot and saved as `<file>-*.png`; its drawing measured against `pack`'s `frame` cut. */
  async function measure(pack: SkinPack, frame: "main" | "chat", file: string, href: string): Promise<Record<string, unknown>> {
    await waitUntil(
      async () => (await browser.execute('var l = document.getElementById("rekindle-skin"); return l && l.sheet ? l.getAttribute("href") : "";')) === href,
      NET_WAIT_MS,
      `${file} never applied ${pack.name}'s sheet`,
    );
    const viewport = (await browser.execute(() => ({ width: window.innerWidth, height: window.innerHeight }))) as { width: number; height: number };
    const dir = path.join(OUT, pack.file);
    mkdirSync(dir, { recursive: true });
    await browser.saveScreenshot(path.join(dir, `${file}-raw.png`));
    const img = decodePng(readFileSync(path.join(dir, `${file}-raw.png`)));
    writeFileSync(path.join(dir, `${file}-over-grey.png`), encodePng(over(img, [128, 128, 128])));
    writeFileSync(path.join(dir, `${file}-over-magenta.png`), encodePng(over(img, [255, 0, 255])));
    const scale = Math.round((img.width * 1000) / viewport.width);
    const cut = silhouette(pack.file, frame, viewport.width, viewport.height, scale);
    const found: Record<string, unknown> = {};
    const where = `${file} ${viewport.width}x${viewport.height}`;
    const cutOff = disagreements(img, cut);
    if (cutOff.outsideDrawn + cutOff.insideClear !== 0) found[`${where} at ${scale}‰`] = cutOff;
    // The content box is the pack's content region exactly; the profile
    // header sits in the pack's header region when it lays one, else in
    // the content box (plan SK0-L1).
    if (cut.content) {
      const page = await pageBoxes();
      const content = offBy(page.content, cut.content);
      if (content) found[`${where} content box`] = content;
      if (frame === "main") {
        // The friends list shows: a collapsed content box hides every row (SK0, Alloy and XGlow 2026-11-20).
        if (page.rows < 1) found[`${where} friends list`] = "no .buddy-item row shows inside the content box";
        if (!page.header) found[`${where} header`] = "no .identity-bar in the buddy list";
        else if (cut.header) {
          const header = offBy(page.header, cut.header);
          if (header) found[`${where} header region`] = header;
        } else {
          const c = cut.content;
          const h = page.header;
          const inside = h.x >= c.x - 1 && h.y >= c.y - 1 && h.x + h.width <= c.x + c.width + 1 && h.y + h.height <= c.y + c.height + 1;
          if (!inside) found[`${where} header in content`] = `${JSON.stringify(h)} is not inside ${JSON.stringify(c)}`;
        }
      }
    }
    return found;
  }

  /** Plan SK1-S1: where the skin lays the title strip, each shown title button is the pack's image at its own size with no glyph or text over it, and no "Rekindle" label is painted. */
  async function titleStrip(file: string): Promise<Record<string, unknown>> {
    const got = (await browser.execute(() => {
      const bad: string[] = [];
      const logo = document.querySelector(".xfire-titlebar-logo");
      if (!logo || getComputedStyle(logo).display === "none") return { laid: false, bad };
      const text = document.querySelector(".xfire-titlebar-text");
      if (text && getComputedStyle(text).display !== "none" && (text.textContent ?? "").trim() === "Rekindle") bad.push("the Rekindle label is painted");
      for (const b of Array.from(document.querySelectorAll<HTMLElement>(".xfire-titlebar-btn"))) {
        const s = getComputedStyle(b);
        if (s.display === "none") continue;
        const glyph = b.querySelector(".xfire-titlebar-glyph");
        if (!s.backgroundImage.startsWith('url("skin://')) bad.push(`${b.className}: background ${s.backgroundImage}`);
        if (s.backgroundSize !== `${s.width} ${s.height}`) bad.push(`${b.className}: art ${s.backgroundSize} in a ${s.width} x ${s.height} box`);
        if (glyph && getComputedStyle(glyph).display !== "none") bad.push(`${b.className}: glyph shown`);
        if (b.innerText.trim() !== "") bad.push(`${b.className}: text "${b.innerText.trim()}"`);
        if (!b.getAttribute("aria-label")) bad.push(`${b.className}: no accessible name`);
      }
      return { laid: true, bad };
    })) as { laid: boolean; bad: string[] };
    return got.bad.length === 0 ? {} : { [`${file} title strip`]: got.bad };
  }

  for (const pack of PACKS) {
    it(`${pack.name}: the buddy list and a chat window are drawn exactly to the skin's cut`, async () => {
      const href = await apply(pack);
      const found: Record<string, unknown> = {};
      await switchTo(browser, BUDDY);
      Object.assign(found, await measure(pack, "main", "buddy-list", href));
      Object.assign(found, await titleStrip("buddy-list"));
      if (!NATIVE_WAYLAND) {
        // A resize recomputes the cut (the shaper reshapes on Resized).
        await sizeWindow(browser, { width: BUDDY_LIST.def.width + 60, height: BUDDY_LIST.def.height + 80 });
        Object.assign(found, await measure(pack, "main", "buddy-list-resized", href));
        await sizeWindow(browser, BUDDY_LIST.def);
      }
      await browser.switchToWindow(handles.chat);
      Object.assign(found, await measure(pack, "chat", "chat", href));
      expect(found).toEqual({});
    });
  }

  it("X11: the server's input and bounding shapes match the cut, and a click outside it passes through", function () {
    if (process.platform !== "linux") this.skip();
    // Owner's direction: only in a nested X server, never the session's
    // display; the tools are not on the Linux laptop yet.
    console.log("[skip] needs Xvfb and an XTEST client (xdotool) with xcb for the nested display: not installed on this machine");
    this.skip();
  });
});
