// Owner (2026-11-26, correcting TAB1): a pack's side tab protrudes beside
// its frame, as the pack lays it out (Symbiosis `IndentRight=13`) and as
// Xfire drew it: the 13x115 `MainInfoViewButton` sits in a 13 px column at
// the window's right, and the column above and below it is transparent
// (alpha 0) and click-through (the window shaper, src-tauri windows/shape).
// TAB1 took that transparency, which a screenshot viewer shows as white,
// for a gap and moved the tab onto the frame; it is reverted. For every
// built-in pack's main and chat windows:
//   1. the frame (the union of the drawn nodes that paint art, the tab
//      aside) reaches `.skin-tree`'s left, top and bottom edges (±1), and
//      its right edge too, except where the pack hangs a tab: there it stops
//      the tab's width short;
//   2. every pixel of the window's leftmost and rightmost 16 px agrees with
//      the shaper's own silhouette (`cargo xtask skin-silhouette`, shared
//      helpers in e2e/app/silhouette.ts): drawn inside it, clear outside it,
//      each mismatch named by its coordinates. A pack's own transparent
//      shape at an edge (Combat's bottom-right channel) is outside it, so it
//      is expected; a CSS-side gap or misplacement is not. A forced gap (a
//      frame art layer hidden) must fail this check (the last test);
//   3. a window with a tab: its left points are painted, the tab's own
//      pixels are opaque, and the tab's column above and below it (the same
//      right points, clear of the tab) is alpha 0; the tab's right edge is
//      the window's and it is 115 tall (its press area may reach back over
//      the frame: Symbiosis serves target [-5, 0, 18, 115], art 13 wide).
// A pack whose frame stops short of an edge with no tab recorded here fails,
// naming the gap. (Symbiosis's minimum width is proven where it is computed:
// a programmatic resize is not bound by a window's content minimum on macOS.)
// Kept: `.skin-tree` is the window's inner box, and no node lives outside it.
// Logged as `[tab]`; each window shot, and its right 40 px cut from the same
// screenshot the pixels are read from (over a checkerboard, see shots.ts),
// as out/shots/frame-fills-window/crops/<pack>-<window>-right.png.

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, doubleClick, switchTo, waitForHandle } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerSendDm, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, friendRow, newUser, seesMessage, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { decodePng, encodePng, type Rgba } from "../../png.ts";
import { checkered } from "../../shots.ts";
import { disagreements, silhouette } from "../../silhouette.ts";
import { shot } from "../../shots.ts";
import { ACCESSIBLE_VARIANT, builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

/** Every built-in pack (lead: a hand-picked list let Simple X's tab go
 *  unchecked); the accessible variant shares Symbiosis's layout. */
const PACKS = builtinPacks()
  .filter((p) => p.file !== ACCESSIBLE_VARIANT.file)
  .map((p) => p.file);
/** The side tab a pack hangs off its main window's right edge: its node,
 *  its art's width (the column it protrudes in) and its height. Its press
 *  area may reach further back over the frame (Symbiosis serves
 *  `target [-5, 0, 18, 115] art [5, 0, 13, 115]`). */
const TABS: Record<string, { node: string; artWidth: number; height: number }> = {
  symbiosis: { node: "MainInfoViewButton", artWidth: 13, height: 115 },
  "simple-x": { node: "InfoViewButton", artWidth: 13, height: 60 },
};

const CROPS = path.join(import.meta.dirname, "..", "..", "out", "shots", "frame-fills-window", "crops");

type Box = [number, number, number, number];

interface Measure {
  window: [number, number];
  tree: Box;
  /** The union of the drawn nodes that paint art, the tab aside, as [left, top, right, bottom]. */
  art: Box;
  tab: Box | null;
  /** Drawn nodes whose box reaches outside `.skin-tree`. */
  outside: string[];
}

/** The current window's pack tree, its art's extent, the tab `tabNode` (if any) and what reaches outside. */
async function measure(tabNode: string): Promise<Measure> {
  return (await browser.execute((tabId: string) => {
    const edges = (r: DOMRect): [number, number, number, number] => [Math.round(r.left), Math.round(r.top), Math.round(r.right), Math.round(r.bottom)];
    const tree = document.querySelector(".skin-tree");
    const t = tree?.getBoundingClientRect();
    const drawn = Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")).filter(
      (n) => !n.closest("[hidden]") && n.getBoundingClientRect().width > 0,
    );
    const art = drawn.filter((n) => n.dataset.skinNode !== tabId && getComputedStyle(n).backgroundImage !== "none");
    const union = art.reduce<[number, number, number, number] | null>((u, n) => {
      const e = edges(n.getBoundingClientRect());
      return u ? [Math.min(u[0], e[0]), Math.min(u[1], e[1]), Math.max(u[2], e[2]), Math.max(u[3], e[3])] : e;
    }, null);
    const tab = tabId ? document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${tabId}"]`) : null;
    const outside = t
      ? drawn
          .filter((n) => {
            const r = n.getBoundingClientRect();
            return r.right > t.right + 0.5 || r.left < t.left - 0.5 || r.bottom > t.bottom + 0.5 || r.top < t.top - 0.5;
          })
          .map((n) => `${n.dataset.skinNode} ${edges(n.getBoundingClientRect()).join(",")}`)
      : [];
    return {
      window: [window.innerWidth, window.innerHeight],
      tree: t ? edges(t) : [0, 0, 0, 0],
      art: union ?? [0, 0, 0, 0],
      tab: tab ? edges(tab.getBoundingClientRect()) : null,
      outside,
    };
  }, tabNode)) as Measure;
}

/** The window's screenshot, the alpha at each probe point (CSS px; `extra`
 *  adds points), and its right 40 px saved as `crop`, over a checkerboard. */
async function probe(w: number, h: number, crop: string, extra: [number, number][] = []): Promise<{ px: Record<string, number>; img: Rgba; scale: number }> {
  const img = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const scale = img.width / w;
  const alpha = (x: number, y: number): number => img.data[(Math.round(y * scale) * img.width + Math.round(x * scale)) * 4 + 3];
  const out: Record<string, number> = {};
  for (const x of [w - 3, w - 10, 2, 9]) for (const y of [60, h - 60]) out[`${x},${y}`] = alpha(x, y);
  for (const [x, y] of extra) out[`${x},${y}`] = alpha(x, y);
  const cw = Math.min(img.width, Math.round(40 * scale));
  const cut: Rgba = { width: cw, height: img.height, data: new Uint8Array(cw * img.height * 4) };
  for (let y = 0; y < img.height; y++) {
    const from = (y * img.width + (img.width - cw)) * 4;
    cut.data.set(img.data.subarray(from, from + cw * 4), y * cw * 4);
  }
  mkdirSync(CROPS, { recursive: true });
  writeFileSync(path.join(CROPS, `${crop}-right.png`), encodePng(checkered(cut)));
  return { px: out, img, scale };
}

/** Measure, log, shoot and require the frame, the paint and the tab for the current window. */
/** Points whose painters are logged, per pack window (lead: StealthX's two
 *  solid blue rectangles at the bottom corners, x 112-135 and 377-400,
 *  y 555-615, are not its art). */
const PAINTER_POINTS: Record<string, [number, number][]> = {
  "stealthx-main": [[123, 585], [388, 585]],
};

/** Every element at (x, y), topmost first: tag, class, node, role, z and paint. */
async function paintersAt(x: number, y: number): Promise<string[]> {
  return (await browser.execute((px: number, py: number) => document.elementsFromPoint(px, py).slice(0, 8).map((e) => {
    const c = getComputedStyle(e);
    const n = (e as HTMLElement).closest<HTMLElement>("[data-skin-node]");
    const art = c.backgroundImage === "none" ? "none" : c.backgroundImage.split("/").pop()?.slice(0, 30);
    const border = c.borderImageSource === "none" ? "none" : c.borderImageSource.split("/").pop()?.slice(0, 30);
    return `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || "-"} node=${n?.dataset.skinNode ?? "-"} role=${n?.dataset.skinRole ?? "-"} state=${n?.dataset.state ?? "-"} z=${c.zIndex} bg=${c.backgroundColor} art=${art} clip=${c.backgroundClip} border-image=${border} op=${c.opacity}`;
  }), x, y)) as string[];
}

/** The page's root paint: html, body, the window root and .skin-frame (the
 *  shape layer's ground lies in the frame's content box, shape/css.rs). */
async function rootPaint(): Promise<string> {
  return (await browser.execute(() => {
    const one = (name: string, e: Element | null): string => {
      if (!e) return `${name}: none`;
      const c = getComputedStyle(e);
      return `${name}: bg=${c.backgroundColor} art=${c.backgroundImage === "none" ? "none" : "set"} clip=${c.backgroundClip} padding=${c.padding} isolation=${c.isolation} display=${c.display} border-image=${c.borderImageSource === "none" ? "none" : "set"}`;
    };
    return [one("html", document.documentElement), one("body", document.body), one("root", document.querySelector(".app-frame")), one("skin-frame", document.querySelector(".skin-frame"))].join(" | ");
  })) as string;
}

async function frameFills(file: string, label: string): Promise<void> {
  for (const [x, y] of PAINTER_POINTS[`${file}-${label}`] ?? []) {
    console.log(`[painters] ${file} ${label} at ${x},${y}: ${JSON.stringify(await paintersAt(x, y))}`);
  }
  const tab = label === "main" ? TABS[file] : undefined;
  const m = await measure(tab?.node ?? "");
  const [w, h] = m.window;
  // The tab's own pixels: its centre, a few px in from the window's edge.
  const tabPoint: [number, number] | null = m.tab ? [w - 6, Math.round((m.tab[1] + m.tab[3]) / 2)] : null;
  const { px, img, scale } = await probe(w, h, `${file}-${label}`, tabPoint ? [tabPoint] : []);
  // The window's edges against the shaper's own silhouette (cargo xtask
  // skin-silhouette): every pixel of the leftmost and rightmost 16 px is
  // drawn exactly where the silhouette is, clear where it is not. A pack's
  // own transparent shape at an edge (Combat's bottom-right channel,
  // Symbiosis's tab column) is outside the silhouette, so it is expected.
  const cut = silhouette(file, label === "chat" ? "chat" : "main", w, h, Math.round(scale * 1000));
  const edge = Math.round(16 * scale);
  // All four edges: the leftmost and rightmost 16 px, and the top and bottom 16 px.
  const off = disagreements(img, cut, [[0, edge], [img.width - edge, img.width]], 12, [[0, edge], [img.height - edge, img.height]]);
  console.log(`[tab] ${file} ${label}: frame=${JSON.stringify(m.art)} tree=${JSON.stringify(m.tree)} window=${JSON.stringify(m.window)} tab=${JSON.stringify(m.tab)} px=${JSON.stringify(px)} outside=${JSON.stringify(m.outside)}`);
  console.log(`[tab] ${file} ${label} edges vs silhouette (16 px on all four sides): drawn outside ${off.outsideDrawn}, clear inside ${off.insideClear}; ${JSON.stringify(off.first)}`);
  await shot(`${file}-${label}-frame`);
  // Lead (Linux, WebKitGTK): a point expected clear but drawn (a tab
  // column's point, or a corner outside the silhouette) is logged with
  // every painter under it and the page's root paint.
  const inside = (x: number, y: number): boolean =>
    !cut.rects || cut.rects.some((r) => x * scale >= r.x && x * scale < r.x + r.width && y * scale >= r.y && y * scale < r.y + r.height);
  const alphaAt = (x: number, y: number): number => img.data[(Math.round(y * scale) * img.width + Math.round(x * scale)) * 4 + 3];
  const corners: [number, number][] = [[1, 1], [w - 2, 1], [1, h - 2], [w - 2, h - 2]];
  const drawnClear = [
    ...(tab ? Object.entries(px).filter(([k]) => Number(k.split(",")[0]) >= w - 10 && k !== tabPoint?.join(",")).filter(([k, a]) => a !== 0 && !inside(...(k.split(",").map(Number) as [number, number]))).map(([k]) => k.split(",").map(Number) as [number, number]) : []),
    ...corners.filter(([x, y]) => !inside(x, y) && alphaAt(x, y) !== 0),
  ];
  // The root paint for every window, so a window drawn clear is the contrast.
  console.log(`[clear-drawn] ${file} ${label} root: ${await rootPaint()} (points drawn where clear: ${drawnClear.length})`);
  if (drawnClear.length > 0) {
    for (const [x, y] of drawnClear) console.log(`[clear-drawn] ${file} ${label} at ${x},${y} alpha ${alphaAt(x, y)}: ${JSON.stringify(await paintersAt(x, y))}`);
  }
  // A pack with no art for this window (Total Chaos has no chat window)
  // leaves it the plain shell window: no tree, and the edge check holds its
  // pixels against the silhouette (the whole rectangle) all the same.
  const laid = m.tree.some((v) => v !== 0);
  if (!laid) console.log(`[tab] ${file} ${label}: no pack tree here; the plain window's edges are checked against its rectangle`);
  if (laid) expect({ window: label, tree: m.tree }).toEqual({ window: label, tree: [0, 0, w, h] });
  expect({ window: label, outside: m.outside }).toEqual({ window: label, outside: [] });
  // Where the frame reaches is the silhouette's to say (StealthX's starts
  // 110 px in, Space Cowboy's below the top): the four edge bands above,
  // pixel by pixel, are the check; an element's box is not its drawn extent.
  expect({ window: label, edgesOffSilhouette: off.outsideDrawn + off.insideClear, first: off.first }).toEqual({ window: label, edgesOffSilhouette: 0, first: [] });
  // The column beside a tab is clear where the silhouette leaves it out
  // (Symbiosis); a pack whose own art fills it (Simple X's white panel down
  // its right side) draws it, and the edge check above holds those pixels.
  const right = Object.entries(px).filter(([k]) => {
    const [x, y] = k.split(",").map(Number);
    return x >= w - 10 && k !== tabPoint?.join(",") && !inside(x, y);
  });
  if (!tab) return;
  if (!m.tab || !tabPoint) throw new Error(`${file}: no drawn ${tab.node}`);
  // Its right edge is the window's and it is the tab's height; its width is
  // its press area's (logged), which may reach back over the frame.
  expect({ tabRight: m.tab[2], height: m.tab[3] - m.tab[1] }).toEqual({ tabRight: w, height: tab.height });
  // The tab is drawn; the column above and below it is transparent.
  expect({ window: label, tabAlpha: px[tabPoint.join(",")] }).toEqual({ window: label, tabAlpha: 255 });
  expect({ window: label, columnNotClear: right.filter(([, a]) => a !== 0) }).toEqual({ window: label, columnNotClear: [] });
}

describe("the pack frame reaches the window's edges; a side tab protrudes beside it in a transparent column (owner)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("framefill"), displayName: "Ari" };
  let peers: Peer[] = [];

  // One friend and one chat window, kept open and re-skinned in place, so
  // every pack's chat frame is checked beside its main window.
  let chat = "";
  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    const myKey = await befriendFleet(browser, me, peers);
    await peerSendDm(peers[0], myKey, "frame check");
    await switchTo(browser, BUDDY);
    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, "Bea"));
    chat = await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h));
    await browser.switchToWindow(chat);
    await seesMessage(browser, me, "frame check");
    await switchTo(browser, BUDDY);
    await openSettings(browser, "Appearance");
  });

  after(async () => {
    await stopFleet(peers);
  });

  for (const file of PACKS) {
    it(`${file}: the main and chat windows' frames reach their edges, a side tab (if any) protruding in a transparent column`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      // Settings stays open from before(), so a pack whose menus live in a
      // popup (StealthX) cannot keep the next switch from Settings.
      await switchTo(browser, SETTINGS);
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      await browser.pause(1500);
      await frameFills(file, "main");
      await switchTo(browser, chat, NET_WAIT_MS);
      await skinShown(browser, href);
      await browser.pause(1500);
      await frameFills(file, "chat");
    });
  }

  // The edge check's mutation proof (lead): a real transparency gap inside
  // the silhouette must be caught. Hiding the right-edge frame art alone
  // leaves other layers drawing there (logged: before, art hidden, and what
  // still paints the point); shifting the window's content 8 px left makes
  // the gap, as a CSS misplacement would. Whether art is drawn at all (colour against
  // the ground) is a different test, not this one.
  it("combat: a frame art layer hidden at the right edge is caught by the edge check", async () => {
    const pack = builtinPacks().find((p) => p.file === "combat");
    if (!pack) throw new Error("no built-in pack combat");
    await switchTo(browser, SETTINGS);
    const href = await applySkin(browser, me, pack.name);
    await switchTo(browser, BUDDY);
    await skinShown(browser, href);
    await browser.pause(1500);
    // A point inside the right-edge frame art, and its colour before.
    const [w, h] = (await browser.execute(() => [window.innerWidth, window.innerHeight])) as [number, number];
    const shoot = async (): Promise<Rgba> => decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
    const at = (img: Rgba, x: number, y: number): string => {
      const s = img.width / w;
      const i = (Math.round(y * s) * img.width + Math.round(x * s)) * 4;
      return `rgba(${img.data[i]},${img.data[i + 1]},${img.data[i + 2]},${img.data[i + 3]})`;
    };
    const probePoint: [number, number] = [w - 6, Math.round(h / 2)];
    const before = at(await shoot(), ...probePoint);
    // 1. Every frame art layer reaching the right edge hidden (the frame
    //    draws one spot from more than one layer). The window's ground
    //    (`.app-frame`, clipped to the silhouette) still fills there, so
    //    this changes colour, not alpha: no gap yet.
    const hidden = (await browser.execute(() => {
      const vw = window.innerWidth;
      const layers = Array.from(document.querySelectorAll<HTMLElement>(".skin-frame i")).filter((i) => {
        const r = i.getBoundingClientRect();
        return r.right >= vw - 1 && r.width > 0 && r.height > 0 && getComputedStyle(i).backgroundImage !== "none";
      });
      for (const layer of layers) {
        layer.style.setProperty("visibility", "hidden", "important");
        layer.dataset.mutated = "1";
      }
      return layers.map((l) => {
        const r = l.getBoundingClientRect();
        return `${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`;
      });
    })) as string[];
    if (hidden.length === 0) throw new Error("combat: no frame art layer at the right edge to hide");
    await browser.pause(500);
    const artHidden = at(await shoot(), ...probePoint);
    // What still paints the probe point with the frame art hidden (a run
    // read the same colour after hiding it and clearing the ground: another
    // layer draws there).
    const painters = (await browser.execute((x: number, y: number) =>
      document
        .elementsFromPoint(x, y)
        .filter((e) => getComputedStyle(e).backgroundImage !== "none" || !/rgba\(\d+, \d+, \d+, 0\)/.test(getComputedStyle(e).backgroundColor))
        .slice(0, 6)
        .map((e) => `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || "-"} (${(e as HTMLElement).closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode ?? "-"}) bg=${getComputedStyle(e).backgroundColor} art=${getComputedStyle(e).backgroundImage === "none" ? "none" : "yes"}`),
    probePoint[0], probePoint[1])) as string[];
    // 2. A real transparency gap (lead): the window's content shifted 8 px
    //    left, as a CSS misplacement would; the rightmost 8 px inside the
    //    silhouette is left with nothing behind it, which the check must catch.
    await browser.execute(() => document.querySelector<HTMLElement>(".app-frame")?.style.setProperty("transform", "translateX(-8px)", "important"));
    await browser.pause(500);
    const img = await shoot();
    const gap = at(img, ...probePoint);
    const scale = img.width / w;
    const cut = silhouette("combat", "main", w, h, Math.round(scale * 1000));
    const edge = Math.round(16 * scale);
    const off = disagreements(img, cut, [[0, edge], [img.width - edge, img.width]], 4);
    console.log(`[tab] combat mutation: hid ${hidden.length} frame layers at the right edge ${JSON.stringify(hidden)}; pixel ${probePoint.join(",")} before ${before}, art hidden ${artHidden} (painted by ${JSON.stringify(painters)}), content shifted ${gap}; edges vs silhouette: drawn outside ${off.outsideDrawn}, clear inside ${off.insideClear}; ${JSON.stringify(off.first)}`);
    await shot("combat-main-mutated");
    await browser.execute(() => {
      for (const l of Array.from(document.querySelectorAll<HTMLElement>('.skin-frame i[data-mutated="1"]'))) l.style.removeProperty("visibility");
      document.querySelector<HTMLElement>(".app-frame")?.style.removeProperty("transform");
    });
    expect({ caught: off.insideClear > 0 }).toEqual({ caught: true });
  });

});
