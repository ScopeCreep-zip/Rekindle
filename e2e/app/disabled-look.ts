// The owner's rule (lead, CR1): a disabled control is visibly disabled. How
// many pixels inside a refused node's box change when it is drawn as its
// enabled self (data-state normal, no aria-disabled or data-disabled-by),
// its attributes put back after; under 20 is not visibly disabled.

import { browser } from "@wdio/globals";
import { decodePng, type Rgba } from "./png.ts";
import { saveImage } from "./shots.ts";

/** The computed paint of a node and its art-bearing descendants, for a
 *  before/after comparison (which property changed between two states). */
const PAINT_PROPS = ["opacity", "filter", "background-image", "background-position", "background-color", "outline-style", "box-shadow", "color", "transform"];

/** `box` (device px) cut out of `img`. */
function crop(img: Rgba, [x, y, w, h]: number[]): Rgba {
  const data = new Uint8Array(w * h * 4);
  for (let row = 0; row < h; row++) {
    for (let col = 0; col < w; col++) {
      const src = ((y + row) * img.width + (x + col)) * 4;
      if (y + row >= img.height || x + col >= img.width || y + row < 0 || x + col < 0) continue;
      data.set(img.data.subarray(src, src + 4), (row * w + col) * 4);
    }
  }
  return { width: w, height: h, data };
}

/** What differs between two crops: the changed pixels' bounding box (crop
 *  px), the mean and max channel delta over them, how many changed alpha,
 *  and a diff image (|delta| x4, opaque). */
function diffOf(a: Rgba, b: Rgba): { summary: string; image: Rgba } {
  const image: Rgba = { width: a.width, height: a.height, data: new Uint8Array(a.data.length) };
  let n = 0, sum = 0, max = 0, alpha = 0, x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  for (let i = 0; i < a.width * a.height; i++) {
    const d = [0, 1, 2].map((k) => Math.abs(a.data[i * 4 + k] - b.data[i * 4 + k]));
    const total = d[0] + d[1] + d[2];
    for (let k = 0; k < 3; k++) image.data[i * 4 + k] = Math.min(255, d[k] * 4);
    image.data[i * 4 + 3] = 255;
    if (total <= 48) continue;
    n++; sum += total / 3; max = Math.max(max, ...d);
    if (a.data[i * 4 + 3] !== b.data[i * 4 + 3]) alpha++;
    const x = i % a.width, y = Math.floor(i / a.width);
    x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y);
  }
  const box = n ? `[${x0},${y0}..${x1},${y1}] of ${a.width}x${a.height}` : "none";
  return { summary: `changed ${n}, box ${box}, mean delta ${n ? Math.round(sum / n) : 0}, max ${max}, alpha changed ${alpha}`, image };
}

/**
 * How many pixels inside each refused node's box change when it is drawn as
 * its enabled self instead (its attributes put back after), by node.
 */
export async function disabledLook(ids: string[]): Promise<Record<string, number>> {
  return Object.fromEntries(Object.entries(await disabledLookWithBoxes(ids)).map(([id, v]) => [id, v.changed]));
}

/**
 * As `disabledLook`, with each node's box (CSS px), so a node that changes
 * nothing can be found on the shot. `inactive` measures instead the nodes a
 * window that lost focus drew in its `windowInactive` state (not refused).
 */
export async function disabledLookWithBoxes(ids: string[], inactive = false): Promise<Record<string, { changed: number; box: number[]; why?: string }>> {
  const width = (await browser.execute(() => innerWidth)) as number;
  const served = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const scale = served.width / width;
  const out: Record<string, { changed: number; box: number[]; why?: string }> = {};
  const paint = async (): Promise<string[]> =>
    (await browser.execute((props: string[]) => {
      const n = document.querySelector<HTMLElement>('[data-cr1-look="1"]');
      if (!n) return [];
      return [n, ...Array.from(n.querySelectorAll<HTMLElement>("*"))].map((e, i) => {
        const c = getComputedStyle(e);
        return `${i}:${e.tagName.toLowerCase()}.${e.className || "-"} ${props.map((p) => `${p}=${c.getPropertyValue(p).slice(0, 60)}`).join(" ")} hover=${e.matches(":hover")} focus=${e.matches(":focus-visible")}`;
      });
    }, PAINT_PROPS)) as string[];
  for (const id of ids) {
    const box = (await browser.execute((node: string, inactiveOnly: boolean) => {
      const drawn = (sel: string): HTMLElement | undefined =>
        Array.from(document.querySelectorAll<HTMLElement>(sel)).find((e) => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0);
      const n = drawn(inactiveOnly ? `.skin-tree [data-skin-node="${node}"][data-state-cause="windowInactive"]` : `.skin-tree [data-skin-node="${node}"][aria-disabled="true"]`);
      if (!n) return null;
      n.setAttribute("data-cr1-look", "1");
      const r = n.getBoundingClientRect();
      const keep = { state: n.getAttribute("data-state"), by: n.getAttribute("data-disabled-by"), cause: n.getAttribute("data-state-cause"), aria: n.getAttribute("aria-disabled") };
      (window as unknown as { __cr1Keep: unknown }).__cr1Keep = keep;
      n.setAttribute("data-state", "normal");
      n.removeAttribute("aria-disabled");
      n.removeAttribute("data-disabled-by");
      n.removeAttribute("data-state-cause");
      return [r.x, r.y, r.width, r.height];
    }, id, inactive)) as number[] | null;
    if (!box) continue;
    const enabled = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
    const normalPaint = inactive ? await paint() : [];
    await browser.execute(() => {
      const n = document.querySelector<HTMLElement>('[data-cr1-look="1"]');
      const keep = (window as unknown as { __cr1Keep: { state: string | null; by: string | null; cause: string | null; aria: string | null } }).__cr1Keep;
      if (!n) return;
      if (keep.state !== null) n.setAttribute("data-state", keep.state);
      if (keep.aria !== null) n.setAttribute("aria-disabled", keep.aria);
      if (keep.by !== null) n.setAttribute("data-disabled-by", keep.by);
      if (keep.cause !== null) n.setAttribute("data-state-cause", keep.cause);
      n.removeAttribute("data-cr1-look");
    });
    const [x, y, w, h] = box.map((v) => Math.round(v * scale));
    let changed = 0;
    for (let row = Math.max(0, y); row < Math.min(served.height, y + h); row++) {
      for (let col = Math.max(0, x); col < Math.min(served.width, x + w); col++) {
        const i = (row * served.width + col) * 4;
        if (Math.abs(served.data[i] - enabled.data[i]) + Math.abs(served.data[i + 1] - enabled.data[i + 1]) + Math.abs(served.data[i + 2] - enabled.data[i + 2]) > 48) changed++;
      }
    }
    out[id] = { changed, box: box.map(Math.round) };
    if (inactive) {
      // Which pixels differ, and why: the node's paint in its served state
      // (windowInactive) against its Normal self, property by property.
      await browser.execute((node: string) => {
        const n = Array.from(document.querySelectorAll<HTMLElement>(`.skin-tree [data-skin-node="${node}"][data-state-cause="windowInactive"]`)).find((e) => e.getBoundingClientRect().width > 0);
        n?.setAttribute("data-cr1-look", "1");
      }, id);
      const servedPaint = await paint();
      await browser.execute(() => document.querySelector('[data-cr1-look="1"]')?.removeAttribute("data-cr1-look"));
      const changedProps = servedPaint.map((line, i) => (line === normalPaint[i] ? null : `${line}  ->  ${normalPaint[i] ?? "(absent)"}`)).filter((l) => l !== null);
      const pad = Math.round(4 * scale);
      const region = [x - pad, y - pad, w + 2 * pad, h + 2 * pad];
      const a = crop(served, region);
      const b = crop(enabled, region);
      const d = diffOf(a, b);
      saveImage(`inactive-${id}-served`, a);
      saveImage(`inactive-${id}-normal`, b);
      saveImage(`inactive-${id}-diff`, d.image);
      out[id].why = `${d.summary}; paint differs: ${changedProps.length ? JSON.stringify(changedProps) : "none"}`;
    }
  }
  return out;
}

/**
 * A node's own art where its button is seen (lead): the served image drawn
 * at the node's background size and position, its alpha at the box's
 * centre and the share of the box its opaque pixels (alpha > 200) cover,
 * and every node with art under the centre, topmost first (which one draws
 * the button seen). An opacity dim on a node whose own art is transparent
 * there changes nothing visible.
 */
export async function ownArt(nodeId: string): Promise<string> {
  return (await browser.execute((nid: string) => new Promise<string>((done) => {
    const n = document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${nid}"]`);
    if (!n) return done("no node");
    const cs = getComputedStyle(n);
    const url = /url\("?([^")]+)"?\)/.exec(cs.backgroundImage)?.[1];
    const r = n.getBoundingClientRect();
    const under = document
      .elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)
      .map((e) => e.closest<HTMLElement>("[data-skin-node]"))
      .filter((e, i, all): e is HTMLElement => !!e && all.indexOf(e) === i)
      .map((e) => `${e.dataset.skinNode}(${e.dataset.skinRole}) art=${getComputedStyle(e).backgroundImage.split("/").pop()?.slice(0, 16) ?? "-"}`);
    if (!url) return done(`no art; under: ${JSON.stringify(under)}`);
    // The art comes from the skin:// protocol, another origin: an <img> of
    // it taints a canvas, so its bytes are fetched and decoded as a bitmap.
    // Any failure is said, and a guard answers if nothing does.
    const guard = setTimeout(() => done(`art not decoded within 10 s; under: ${JSON.stringify(under)}`), 10_000);
    const finish = (text: string): void => {
      clearTimeout(guard);
      done(text);
    };
    fetch(url)
      .then((res) => (res.ok ? res.blob() : Promise.reject(new Error(`HTTP ${res.status}`))))
      .then((blob) => createImageBitmap(blob))
      .then((img) => {
        const c = document.createElement("canvas");
        c.width = Math.max(1, Math.round(r.width));
        c.height = Math.max(1, Math.round(r.height));
        const g = c.getContext("2d")!;
        const [px, py] = cs.backgroundPosition.split(" ").map((v) => parseFloat(v) || 0);
        const size = cs.backgroundSize.split(" ").map((v) => parseFloat(v));
        const w = Number.isFinite(size[0]) ? size[0] : img.width;
        const h = Number.isFinite(size[1]) ? size[1] : img.height;
        g.drawImage(img, px, py, w, h);
        const all = g.getImageData(0, 0, c.width, c.height).data;
        let opaque = 0;
        for (let i = 3; i < all.length; i += 4) if (all[i] > 200) opaque++;
        const centre = g.getImageData(Math.floor(c.width / 2), Math.floor(c.height / 2), 1, 1).data[3];
        const cover = Math.round((opaque / (c.width * c.height)) * 100);
        finish(`image ${img.width}x${img.height} at ${cs.backgroundPosition} size ${cs.backgroundSize} over box ${c.width}x${c.height}: alpha at centre ${centre}, opaque cover ${cover}%; under: ${JSON.stringify(under)}`);
      })
      .catch((e: unknown) => finish(`art not read (${String(e).slice(0, 120)}); under: ${JSON.stringify(under)}`));
  }), nodeId)) as string;
}

/**
 * What a node's own art adds on screen, measured without reading the art
 * (the page cannot read skin:// art: an <img> taints a canvas, and fetch is
 * refused): the window is shot with the node shown and with it hidden
 * (`visibility: hidden`, children too), and the pixels that change inside
 * its box are counted, with that count's share of the box. Near 0 means its
 * own art draws nothing there; the button seen is another layer's. Also
 * every element under the box's centre with a background, topmost first,
 * skin nodes or not (the frame's own art included).
 */
export async function artContribution(nodeId: string): Promise<string> {
  const info = (await browser.execute((nid: string) => {
    const n = document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${nid}"]`);
    if (!n) return null;
    const r = n.getBoundingClientRect();
    const under = document
      .elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)
      .filter((e) => getComputedStyle(e).backgroundImage !== "none" || getComputedStyle(e).backgroundColor !== "rgba(0, 0, 0, 0)")
      .slice(0, 8)
      .map((e) => {
        const c = getComputedStyle(e);
        const node = (e as HTMLElement).dataset.skinNode ?? (e as HTMLElement).closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode ?? "-";
        return `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || "-"} (${node}) art=${c.backgroundImage === "none" ? "none" : c.backgroundImage.split("/").pop()?.slice(0, 16)} bg=${c.backgroundColor}`;
      });
    return { box: [r.left, r.top, r.width, r.height], width: window.innerWidth, under };
  }, nodeId)) as { box: number[]; width: number; under: string[] } | null;
  if (!info) return "no node";
  const shoot = async () => decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const shown = await shoot();
  await browser.execute((nid: string) => {
    const n = document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${nid}"]`);
    if (n) n.style.setProperty("visibility", "hidden", "important");
  }, nodeId);
  await browser.pause(300);
  const hidden = await shoot();
  await browser.execute((nid: string) => {
    document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${nid}"]`)?.style.removeProperty("visibility");
  }, nodeId);
  const scale = shown.width / info.width;
  const [x0, y0, w, h] = info.box.map((v) => Math.round(v * scale));
  let changed = 0;
  for (let y = Math.max(0, y0); y < Math.min(shown.height, y0 + h); y++) {
    for (let x = Math.max(0, x0); x < Math.min(shown.width, x0 + w); x++) {
      const i = (y * shown.width + x) * 4;
      if (Math.abs(shown.data[i] - hidden.data[i]) + Math.abs(shown.data[i + 1] - hidden.data[i + 1]) + Math.abs(shown.data[i + 2] - hidden.data[i + 2]) > 30) changed++;
    }
  }
  const area = Math.max(1, w * h);
  return `box ${info.box.map((v) => Math.round(v)).join(",")}: hiding it changes ${changed} of ${area} px (${Math.round((changed / area) * 100)}%); under its centre: ${JSON.stringify(info.under)}`;
}
