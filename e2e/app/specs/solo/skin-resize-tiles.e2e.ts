// SKC4c's real-app proof: a pack's tiles join at every window size. The
// importer chains a resize tile on a moving sibling's edge and paints
// stacked title rows once (Total Chaos declares one title row with a 1 px
// left outdent; the merged pack painted a second, un-outdented row). In
// Total Chaos and in Alloy, the buddy list at its served minimum (squeezed
// below it, then the pack applied: the shell grows it to the minimum,
// window-fit) and resized larger:
//   1. its four 16 px edges agree with the shaper's own silhouette at that
//      size (cargo xtask skin-silhouette, silhouette.ts): no gap the CSS
//      leaves, no art outside the cut;
//   2. no row is painted twice: no displayed frame-layer tile
//      (`.skin-frame > i`) shares a band with a tile drawing an image a
//      painted tree node draws over an overlapping box (frames are served at
//      their content hash, so the same art is the same URL). The pre-fix
//      importer served Total Chaos's title row both as the tree's
//      `MainTitleBar` (OutdentLeft 1) and as three un-outdented frame tiles
//      52 px tall at x 0 (left cap, middle repeat, right cap; the middle and
//      right are the tree's own images, the left cap the uncropped one): a
//      1 px difference no pixel check holds reliably, and a serve-side error
//      the silhouette shares, so this structural check is the one that sees it.
//   3. no two painted skin nodes draw the same image at the same position
//      and size over overlapping boxes.

import { browser, expect } from "@wdio/globals";
import { switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { decodePng } from "../../png.ts";
import { shot } from "../../shots.ts";
import { disagreements, silhouette } from "../../silhouette.ts";

const PACKS: { name: string; file: string }[] = [
  { name: "Total Chaos", file: "total-chaos" },
  { name: "Alloy", file: "alloy" },
];

/** The buddy list's inner size, once it holds still for a second. */
async function settledSize(): Promise<[number, number]> {
  let last = "";
  let still = 0;
  let size: [number, number] = [0, 0];
  await waitUntil(
    async () => {
      size = (await browser.execute(() => [window.innerWidth, window.innerHeight])) as [
        number,
        number,
      ];
      const now = size.join("x");
      still = now === last ? still + 1 : 0;
      last = now;
      return still >= 4;
    },
    15_000,
    "the window never held its size"
  );
  return size;
}

/**
 * Frame tiles of a row the tree also paints. The Host serves each image at its
 * content hash (`regions::css::href`, `{base}/{blake3}.{ext}`) and frame slots
 * carry no source name, so the same art is the same URL: a displayed frame
 * tile whose background is an image a painted tree node (or its `::before` /
 * `::after` layer) draws, over an overlapping box, repeats that row. A row's
 * tiles share one band (top and height), and a row's pieces need not all be
 * byte-identical to the tree's (Total Chaos's left cap is the uncropped
 * 75x52 where the tree draws a 74x52 crop), so every displayed tile in a band
 * holding such a tile is reported: the whole row is painted twice.
 */
async function frameRepeatsTreeArt(): Promise<string[]> {
  return (await browser.execute(() => {
    const urls = (bg: string) => Array.from(bg.matchAll(/url\("?([^")]+)"?\)/g), (m) => m[1]);
    const shown = (el: HTMLElement) => {
      const st = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return st.display !== "none" && st.visibility !== "hidden" && r.width > 0 && r.height > 0;
    };
    const overlap = (a: DOMRect, b: DOMRect) =>
      Math.min(a.right, b.right) > Math.max(a.left, b.left) &&
      Math.min(a.bottom, b.bottom) > Math.max(a.top, b.top);
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]"))
      .filter(shown)
      .map((el) => ({
        node: el.getAttribute("data-skin-node") ?? "",
        urls: [
          ...urls(getComputedStyle(el).backgroundImage),
          ...urls(getComputedStyle(el, "::before").backgroundImage),
          ...urls(getComputedStyle(el, "::after").backgroundImage),
        ],
        rect: el.getBoundingClientRect(),
      }))
      .filter((n) => n.urls.length > 0);
    const tiles = Array.from(document.querySelectorAll<HTMLElement>(".skin-frame > i"))
      .map((el, i) => ({ slot: i + 1, el }))
      .filter((t) => shown(t.el))
      .map((t) => ({
        slot: t.slot,
        art: urls(getComputedStyle(t.el).backgroundImage),
        rect: t.el.getBoundingClientRect(),
      }))
      .filter((t) => t.art.length > 0);
    const hash = (u: string) => (/([0-9a-f]{8})[0-9a-f]*\.(?:png|gif)$/.exec(u) ?? [])[1] ?? u.slice(-12);
    const band = (r: DOMRect) => `${Math.round(r.top)}:${Math.round(r.height)}`;
    const repeated = new Map<string, string>();
    for (const t of tiles) {
      for (const n of nodes) {
        const same = t.art.find((u) => n.urls.includes(u));
        if (same && overlap(t.rect, n.rect)) {
          repeated.set(band(t.rect), n.node);
          break;
        }
      }
    }
    return tiles
      .filter((t) => repeated.has(band(t.rect)))
      .map((t) => `frame slot ${t.slot} (${hash(t.art[0])}, band ${band(t.rect)}) repeats ${repeated.get(band(t.rect))}`);
  })) as string[];
}

/** Pairs of painted skin nodes drawing the same art in the same place. */
async function doubledArt(): Promise<string[]> {
  return (await browser.execute(() => {
    const painted = Array.from(
      document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")
    )
      .map((el) => {
        const st = getComputedStyle(el);
        const r = el.getBoundingClientRect();
        return {
          node: el.getAttribute("data-skin-node") ?? "",
          art: st.backgroundImage,
          at: `${st.backgroundPosition}|${st.backgroundSize}|${st.backgroundRepeat}`,
          r,
        };
      })
      .filter((p) => p.art !== "none" && p.r.width > 0 && p.r.height > 0);
    const out: string[] = [];
    for (let i = 0; i < painted.length; i++) {
      for (let j = i + 1; j < painted.length; j++) {
        const a = painted[i];
        const b = painted[j];
        if (a.art !== b.art || a.at !== b.at) continue;
        const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
        const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
        if (w > 0 && h > 0)
          out.push(`${a.node} over ${b.node} (${Math.round(w)}x${Math.round(h)} px)`);
      }
    }
    return out;
  })) as string[];
}

describe("a pack's tiles join at its minimum and when resized larger (SKC4c)", function () {
  this.timeout(900_000);
  const ada: User = { ...newUser("resize-tiles"), displayName: "Ada" };

  it("Total Chaos and Alloy: edges match the silhouette and no art is painted twice, at both sizes", async () => {
    await createIdentity(browser, ada);
    const faults: string[] = [];
    for (const pack of PACKS) {
      // Squeezed below any pack's minimum first: applying the pack grows the
      // window to the minimum the Host serves for it (window-fit; macOS does
      // not clamp WebDriver's resize, and a window squeezed after the skin is
      // applied is not grown back until a skin is applied again).
      await switchTo(browser, BUDDY);
      await browser.setWindowSize(200, 300);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, ada, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      const min = await settledSize();
      if (min[0] <= 200 || min[1] <= 300)
        faults.push(`${pack.name}: the window never grew to its minimum (${min.join("x")})`);
      const sizes: [string, () => Promise<void>][] = [
        ["minimum", async () => undefined],
        ["larger", async () => browser.setWindowSize(min[0] + 160, min[1] + 120)],
      ];
      for (const [label, resize] of sizes) {
        await resize();
        const [w, h] = await settledSize();
        const img = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
        const scale = img.width / w;
        const cut = silhouette(pack.file, "main", w, h, Math.round(scale * 1000));
        const edge = Math.round(16 * scale);
        const off = disagreements(
          img,
          cut,
          [
            [0, edge],
            [img.width - edge, img.width],
          ],
          12,
          [
            [0, edge],
            [img.height - edge, img.height],
          ]
        );
        const doubled = await doubledArt();
        const twice = await frameRepeatsTreeArt();
        console.log(
          `[resize-tiles] ${pack.name} ${label} ${w}x${h}: edges drawn outside ${off.outsideDrawn}, clear inside ${off.insideClear} ${JSON.stringify(off.first)}; doubled art ${JSON.stringify(doubled)}; frame tiles repeating tree art ${JSON.stringify(twice)}`
        );
        await shot(`${pack.file}-${label}`);
        if (off.outsideDrawn + off.insideClear > 0)
          faults.push(`${pack.name} ${label}: edges ${off.outsideDrawn}/${off.insideClear}`);
        if (doubled.length > 0) faults.push(`${pack.name} ${label}: doubled ${doubled.join(", ")}`);
        if (twice.length > 0)
          faults.push(`${pack.name} ${label}: frame tiles repeat tree art: ${twice.join(", ")}`);
      }
    }
    expect(faults).toEqual([]);
  });
});
