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
//      (`.skin-frame > i`) draws a `row-*`/`node-*` image for a row the skin
//      tree also paints. The pre-fix importer served Total Chaos's title row
//      both as the tree's `MainTitleBar` (OutdentLeft 1) and as three
//      un-outdented frame tiles (`row-maintitlebar-left`, `node-maintitlebar-
//      middle`, `node-maintitlebar-right`, 52 px tall at x 0): a 1 px
//      difference no pixel check holds reliably, and a serve-side error the
//      silhouette shares, so this structural check is the one that sees it.
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

/** Displayed frame tiles drawing a row the skin tree also paints: a tile's
 *  `row-<row>-*` / `node-<row>-*` image names a node the tree lays (case
 *  aside: `MainTitleBar` is `maintitlebar`). */
async function rowTilesTwice(): Promise<string[]> {
  return (await browser.execute(() => {
    const treeNodes = new Set(
      Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")).map((el) =>
        (el.getAttribute("data-skin-node") ?? "").toLowerCase()
      )
    );
    const out: string[] = [];
    for (const tile of Array.from(document.querySelectorAll<HTMLElement>(".skin-frame > i"))) {
      const st = getComputedStyle(tile);
      if (st.display === "none" || st.backgroundImage === "none") continue;
      const file = /([^/"]+)\.png/.exec(st.backgroundImage)?.[1] ?? "";
      const row = /^(?:row|node)-([a-z0-9]+)-/.exec(file)?.[1];
      if (row && treeNodes.has(row)) out.push(file);
    }
    return out;
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
        const twice = await rowTilesTwice();
        console.log(
          `[resize-tiles] ${pack.name} ${label} ${w}x${h}: edges drawn outside ${off.outsideDrawn}, clear inside ${off.insideClear} ${JSON.stringify(off.first)}; doubled art ${JSON.stringify(doubled)}; frame tiles repeating a tree row ${JSON.stringify(twice)}`
        );
        await shot(`${pack.file}-${label}`);
        if (off.outsideDrawn + off.insideClear > 0)
          faults.push(`${pack.name} ${label}: edges ${off.outsideDrawn}/${off.insideClear}`);
        if (doubled.length > 0) faults.push(`${pack.name} ${label}: doubled ${doubled.join(", ")}`);
        if (twice.length > 0)
          faults.push(`${pack.name} ${label}: frame tiles repeat a tree row: ${twice.join(", ")}`);
      }
    }
    expect(faults).toEqual([]);
  });
});
