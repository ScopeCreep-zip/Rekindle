// SKC4c's real-app proof: a pack's tiles join at every window size. The
// importer chains a resize tile on a moving sibling's edge and paints
// stacked title rows once (Total Chaos declares one title row with a 1 px
// left outdent; the merged pack painted a second, un-outdented row). In
// every built-in pack the Host lists, the buddy list at its served minimum (squeezed
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
import { decodePng, type Rgba } from "../../png.ts";
import { saveImage, shot } from "../../shots.ts";
import { disagreements, silhouette } from "../../silhouette.ts";

/**
 * Every built-in pack the Host ships, as its library lists it (`list_skins`):
 * the predicates are general, so the spec walks them all rather than a list of
 * the packs a fault was first seen in. `file` is the pack id the silhouette
 * tool reads (`builtin:<id>`).
 */
async function builtinPacks(): Promise<{ name: string; file: string }[]> {
  const library = (await browser.execute(async () => {
    const { invoke } = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(cmd: string): Promise<{ value: { skins: { id: string; name: string }[] } }>;
        };
      }
    ).__TAURI_INTERNALS__;
    return invoke("list_skins");
  })) as { value: { skins: { id: string; name: string }[] } };
  // A Host call answers `Done { value, undo }` (dto/command.rs).
  return library.value.skins
    .filter((s) => s.id.startsWith("builtin:"))
    .map((s) => ({ name: s.name, file: s.id.slice("builtin:".length) }));
}

/**
 * The minimum the Host serves for `file`'s buddy list (`get_skin`'s Main shell,
 * logical px with the frame's outsets: what the desktop's fit holds the window
 * to), or null when the pack lays no tree for it.
 */
async function servedMinimum(file: string): Promise<[number, number] | null> {
  const shell = (await browser.execute(async (id: string) => {
    const { invoke } = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            cmd: string,
            args: unknown
          ): Promise<{
            value: {
              shells?: { window: string; popup?: unknown; minWidth: number; minHeight: number }[];
            };
          }>;
        };
      }
    ).__TAURI_INTERNALS__;
    const applied = await invoke("get_skin", { id });
    const main = (applied.value.shells ?? []).find(
      (s) => s.window === "main" && s.popup === undefined
    );
    return main ? [main.minWidth, main.minHeight] : null;
  }, `builtin:${file}`)) as [number, number] | null;
  return shell;
}

/** Whether `got` is `want` within the 2 px a fractional scale rounds to. */
function near(got: [number, number], want: [number, number]): boolean {
  return Math.abs(got[0] - want[0]) <= 2 && Math.abs(got[1] - want[1]) <= 2;
}

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
 * The boxes the Host serves the Rekindle mark in (the brand pass replaces each
 * Xfire logo node's art with the mark, `skin_tree/brand.rs`), and every
 * displayed frame tile drawn inside one: no Xfire brand anywhere the user can
 * see it, so nothing of the pack's own may sit under the mark where its glyph
 * is transparent. A tile inside a mark's box exists only to paint that box (a
 * full-window ground merely passes under it and is not inside), so any is a
 * fault.
 */
async function frameUnderBrand(): Promise<{
  marks: { node: string; box: number[] }[];
  under: string[];
}> {
  return (await browser.execute(() => {
    const shown = (el: Element) => {
      const st = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return st.display !== "none" && st.visibility !== "hidden" && r.width > 0 && r.height > 0;
    };
    // Each mark names only its own node (the nearest skin node holding the
    // SVG), never the layout ancestors that contain it.
    // The mark's own cover (the glyph wrapper the brand pass lays over the
    // pack's logo art, skin-tree.css's .skin-node-glyph[data-cover]), named
    // by its nearest skin node; opaque brand labels elsewhere cannot leak.
    const marks = Array.from(
      document.querySelectorAll<HTMLElement>(".skin-tree .skin-node-glyph[data-cover]")
    )
      .filter(
        (cover) =>
          cover.querySelector(":scope > svg.skin-node-glyph-brand") !== null && shown(cover)
      )
      .map((cover) => ({ el: cover, owner: cover.closest<HTMLElement>("[data-skin-node]") }))
      .filter((m): m is { el: HTMLElement; owner: HTMLElement } => m.owner !== null)
      .map((m) => ({
        node: m.owner.getAttribute("data-skin-node") ?? "",
        r: m.el.getBoundingClientRect(),
      }));
    const under: string[] = [];
    Array.from(document.querySelectorAll<HTMLElement>(".skin-frame > i")).forEach((tile, slot) => {
      if (!shown(tile) || getComputedStyle(tile).backgroundImage === "none") return;
      const t = tile.getBoundingClientRect();
      for (const m of marks) {
        const inside =
          t.left >= m.r.left - 1 &&
          t.right <= m.r.right + 1 &&
          t.top >= m.r.top - 1 &&
          t.bottom <= m.r.bottom + 1;
        if (inside) {
          under.push(
            `frame slot ${slot + 1} (${Math.round(t.width)}x${Math.round(t.height)}) under the mark of ${m.node}`
          );
          return;
        }
      }
    });
    return {
      marks: marks.map((m) => ({
        node: m.node,
        box: [
          Math.round(m.r.left),
          Math.round(m.r.top),
          Math.round(m.r.width),
          Math.round(m.r.height),
        ],
      })),
      under,
    };
  })) as { marks: { node: string; box: number[] }[]; under: string[] };
}

/**
 * `img` cut to the CSS box `[x, y, w, h]` at device `scale`, each edge rounded
 * and clamped to the image, for a reviewed shot; null when the box lies
 * wholly outside the image.
 */
function cropBox(img: Rgba, box: number[], scale: number): Rgba | null {
  const clamp = (v: number, hi: number) => Math.min(Math.max(Math.round(v), 0), hi);
  const [x, y, w, h] = box;
  const x0 = clamp(x * scale, img.width);
  const y0 = clamp(y * scale, img.height);
  const x1 = clamp((x + w) * scale, img.width);
  const y1 = clamp((y + h) * scale, img.height);
  if (x1 <= x0 || y1 <= y0) return null;
  const cw = x1 - x0;
  const ch = y1 - y0;
  const data = new Uint8Array(cw * ch * 4);
  for (let row = 0; row < ch; row++) {
    const from = ((y0 + row) * img.width + x0) * 4;
    data.set(img.data.subarray(from, from + cw * 4), row * cw * 4);
  }
  return { width: cw, height: ch, data };
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
    // A node's painted layers: its own background over its box, and each
    // generated ::before/::after that is laid (content and display set) over
    // the box its insets give it inside the node.
    type Box = { left: number; top: number; right: number; bottom: number };
    const px = (v: string) => (v.endsWith("px") ? Number.parseFloat(v) : Number.NaN);
    // One axis of an absolutely placed layer inside [lo, hi]: its start and
    // end insets and its size, each applied when set.
    const axis = (
      lo: number,
      hi: number,
      start: number,
      end: number,
      size: number
    ): [number, number] => {
      let a = Number.isNaN(start) ? lo : lo + start;
      let b = Number.isNaN(end) ? hi : hi - end;
      if (!Number.isNaN(size)) {
        if (!Number.isNaN(start)) b = a + size;
        else if (!Number.isNaN(end)) a = b - size;
      }
      return [a, b];
    };
    const pseudoLayer = (
      el: HTMLElement,
      r: DOMRect,
      pseudo: string
    ): { urls: string[]; rect: Box } | null => {
      const ps = getComputedStyle(el, pseudo);
      if (ps.content === "none" || ps.content === "normal" || ps.display === "none") return null;
      if (ps.position !== "absolute" && ps.position !== "fixed")
        return { urls: urls(ps.backgroundImage), rect: r as Box };
      const [left, right] = axis(r.left, r.right, px(ps.left), px(ps.right), px(ps.width));
      const [top, bottom] = axis(r.top, r.bottom, px(ps.top), px(ps.bottom), px(ps.height));
      return { urls: urls(ps.backgroundImage), rect: { left, top, right, bottom } };
    };
    const layers = (el: HTMLElement): { urls: string[]; rect: Box }[] => {
      const r = el.getBoundingClientRect();
      const out = [
        { urls: urls(getComputedStyle(el).backgroundImage), rect: r as Box },
        pseudoLayer(el, r, "::before"),
        pseudoLayer(el, r, "::after"),
      ];
      return out.filter(
        (x): x is { urls: string[]; rect: Box } =>
          x !== null &&
          x.urls.length > 0 &&
          x.rect.right > x.rect.left &&
          x.rect.bottom > x.rect.top
      );
    };
    const overlap = (a: Box, b: Box) =>
      Math.min(a.right, b.right) > Math.max(a.left, b.left) &&
      Math.min(a.bottom, b.bottom) > Math.max(a.top, b.top);
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]"))
      .filter(shown)
      .map((el) => ({
        node: el.getAttribute("data-skin-node") ?? "",
        layers: layers(el),
      }))
      .filter((n) => n.layers.length > 0);
    const tiles = Array.from(document.querySelectorAll<HTMLElement>(".skin-frame > i"))
      .map((el, i) => ({ slot: i + 1, el }))
      .filter((t) => shown(t.el))
      .map((t) => ({
        slot: t.slot,
        art: urls(getComputedStyle(t.el).backgroundImage),
        rect: t.el.getBoundingClientRect(),
      }))
      .filter((t) => t.art.length > 0);
    const hash = (u: string) =>
      (/([0-9a-f]{8})[0-9a-f]*\.(?:png|gif)$/.exec(u) ?? [])[1] ?? u.slice(-12);
    const band = (r: DOMRect) => `${Math.round(r.top)}:${Math.round(r.height)}`;
    const repeated = new Map<string, string>();
    for (const t of tiles) {
      for (const n of nodes) {
        const same = n.layers.some(
          (layer) => t.art.some((u) => layer.urls.includes(u)) && overlap(t.rect, layer.rect)
        );
        if (same) {
          repeated.set(band(t.rect), n.node);
          break;
        }
      }
    }
    return tiles
      .filter((t) => repeated.has(band(t.rect)))
      .map(
        (t) =>
          `frame slot ${t.slot} (${hash(t.art[0])}, band ${band(t.rect)}) repeats ${repeated.get(band(t.rect))}`
      );
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
          shown:
            st.display !== "none" && st.visibility !== "hidden" && el.closest("[hidden]") === null,
          box: [r.left, r.top, r.width, r.height].map((v) => Math.round(v)).join(","),
          r,
        };
      })
      .filter((p) => p.shown && p.art !== "none" && p.r.width > 0 && p.r.height > 0);
    const out: string[] = [];
    for (let i = 0; i < painted.length; i++) {
      for (let j = i + 1; j < painted.length; j++) {
        const a = painted[i];
        const b = painted[j];
        // The same image over the same absolute box: relative positions are
        // not compared (one CSS position over two boxes paints two phases).
        if (a.art === b.art && a.box === b.box)
          out.push(
            `${a.node} over ${b.node} (${Math.round(a.r.width)}x${Math.round(a.r.height)} px)`
          );
      }
    }
    return out;
  })) as string[];
}

/**
 * One pack at its current size: its edges against the shaper's silhouette, art
 * painted twice, frame tiles repeating tree art, and frame art under the
 * Rekindle mark (each mark's box saved as a shot); the faults found.
 */
async function checkSize(pack: { name: string; file: string }, label: string): Promise<string[]> {
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
  const brand = await frameUnderBrand();
  for (const m of brand.marks) {
    const crop = cropBox(img, m.box, scale);
    if (crop) saveImage(`${pack.file}-${label}-mark-${m.node}`, crop);
    else
      console.log(
        `[resize-tiles] ${pack.name} ${label}: the mark of ${m.node} lies outside the shot ${JSON.stringify(m.box)}`
      );
  }
  console.log(
    `[resize-tiles] ${pack.name} ${label} ${w}x${h}: edges drawn outside ${off.outsideDrawn}, clear inside ${off.insideClear} ${JSON.stringify(off.first)}; doubled art ${JSON.stringify(doubled)}; frame tiles repeating tree art ${JSON.stringify(twice)}`
  );
  console.log(
    `[resize-tiles] ${pack.name} ${label}: marks ${JSON.stringify(brand.marks.map((m) => m.node))}; frame under the mark ${JSON.stringify(brand.under)}`
  );
  await shot(`${pack.file}-${label}`);
  const faults: string[] = [];
  if (off.outsideDrawn + off.insideClear > 0)
    faults.push(`${pack.name} ${label}: edges ${off.outsideDrawn}/${off.insideClear}`);
  if (doubled.length > 0) faults.push(`${pack.name} ${label}: doubled ${doubled.join(", ")}`);
  if (twice.length > 0)
    faults.push(`${pack.name} ${label}: frame tiles repeat tree art: ${twice.join(", ")}`);
  if (brand.under.length > 0)
    faults.push(
      `${pack.name} ${label}: frame art under the Rekindle mark: ${brand.under.join(", ")}`
    );
  return faults;
}

describe("a pack's tiles join at its minimum and when resized larger (SKC4c)", function () {
  this.timeout(2_700_000);
  const ada: User = { ...newUser("resize-tiles"), displayName: "Ada" };

  it("every built-in pack: edges match the silhouette and no art is painted twice, at both sizes", async () => {
    await createIdentity(browser, ada);
    const packs = await builtinPacks();
    console.log(
      `[resize-tiles] built-in packs (${packs.length}): ${packs.map((p) => p.file).join(", ")}`
    );
    expect(packs.length).toBeGreaterThan(0);
    const faults: string[] = [];
    // A pack counts as walked only once both sizes were measured: an empty or
    // short walk must fail, never pass with nothing checked.
    const walked: string[] = [];
    for (const pack of packs) {
      try {
        // Squeezed below the pack's served minimum first, then the pack applied:
        // where the squeeze took (an allocation below the minimum), the fit must
        // grow the window to it. Then the window is set to the served minimum
        // and verified there, and resized larger and verified changed, so a
        // pack that was already active (no reapply) or a clamped resize cannot
        // pass for a size that was never laid.
        await switchTo(browser, BUDDY);
        await browser.setWindowSize(200, 300);
        const squeezed = await settledSize();
        await openSettings(browser, "Appearance");
        const href = await applySkin(browser, ada, pack.name);
        await switchTo(browser, BUDDY);
        await skinShown(browser, href);
        const served = await servedMinimum(pack.file);
        const afterApply = await settledSize();
        console.log(
          `[resize-tiles] ${pack.name}: served minimum ${served ? served.join("x") : "none (no tree)"}; squeezed ${squeezed.join("x")}; after apply ${afterApply.join("x")}`
        );
        if (served && (squeezed[0] < served[0] - 2 || squeezed[1] < served[1] - 2)) {
          if (afterApply[0] < served[0] - 2 || afterApply[1] < served[1] - 2)
            faults.push(
              `${pack.name}: the window stayed below its served minimum ${served.join("x")} (${afterApply.join("x")})`
            );
        }
        const min: [number, number] = served ?? afterApply;
        await browser.setWindowSize(min[0], min[1]);
        const atMin = await settledSize();
        if (!near(atMin, min))
          throw new Error(
            `the window did not take its minimum ${min.join("x")} (${atMin.join("x")})`
          );
        faults.push(...(await checkSize(pack, "minimum")));
        await browser.setWindowSize(min[0] + 160, min[1] + 120);
        const larger = await settledSize();
        if (larger[0] <= atMin[0] && larger[1] <= atMin[1])
          throw new Error(`the window did not grow from ${atMin.join("x")} (${larger.join("x")})`);
        faults.push(...(await checkSize(pack, "larger")));
        walked.push(pack.file);
      } catch (e) {
        faults.push(
          `${pack.file}: not walked: ${(e instanceof Error ? e.message : String(e)).split("\n")[0]}`
        );
      }
    }
    console.log(`[resize-tiles] walked ${walked.length} of ${packs.length}: ${walked.join(", ")}`);
    expect(faults).toEqual([]);
    expect(walked).toEqual(packs.map((p) => p.file));
  });
});
