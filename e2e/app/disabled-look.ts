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
