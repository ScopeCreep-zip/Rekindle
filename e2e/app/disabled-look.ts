// The owner's rule (lead, CR1): a disabled control is visibly disabled. How
// many pixels inside a refused node's box change when it is drawn as its
// enabled self (data-state normal, no aria-disabled or data-disabled-by),
// its attributes put back after; under 20 is not visibly disabled.

import { browser } from "@wdio/globals";
import { decodePng } from "./png.ts";

/**
 * How many pixels inside each refused node's box change when it is drawn as
 * its enabled self instead (its attributes put back after), by node.
 */
export async function disabledLook(ids: string[]): Promise<Record<string, number>> {
  return Object.fromEntries(Object.entries(await disabledLookWithBoxes(ids)).map(([id, v]) => [id, v.changed]));
}

/** As `disabledLook`, with each node's box (CSS px), so a node that changes nothing can be found on the shot. */
export async function disabledLookWithBoxes(ids: string[]): Promise<Record<string, { changed: number; box: number[] }>> {
  const width = (await browser.execute(() => innerWidth)) as number;
  const served = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  const scale = served.width / width;
  const out: Record<string, { changed: number; box: number[] }> = {};
  for (const id of ids) {
    const box = (await browser.execute((node: string) => {
      const drawn = (sel: string): HTMLElement | undefined =>
        Array.from(document.querySelectorAll<HTMLElement>(sel)).find((e) => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0);
      const n = drawn(`.skin-tree [data-skin-node="${node}"][aria-disabled="true"]`);
      if (!n) return null;
      n.setAttribute("data-cr1-look", "1");
      const r = n.getBoundingClientRect();
      const keep = { state: n.getAttribute("data-state"), by: n.getAttribute("data-disabled-by") };
      (window as unknown as { __cr1Keep: unknown }).__cr1Keep = keep;
      n.setAttribute("data-state", "normal");
      n.removeAttribute("aria-disabled");
      n.removeAttribute("data-disabled-by");
      return [r.x, r.y, r.width, r.height];
    }, id)) as number[] | null;
    if (!box) continue;
    const enabled = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
    await browser.execute(() => {
      const n = document.querySelector<HTMLElement>('[data-cr1-look="1"]');
      const keep = (window as unknown as { __cr1Keep: { state: string | null; by: string | null } }).__cr1Keep;
      if (!n) return;
      if (keep.state !== null) n.setAttribute("data-state", keep.state);
      n.setAttribute("aria-disabled", "true");
      if (keep.by !== null) n.setAttribute("data-disabled-by", keep.by);
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
