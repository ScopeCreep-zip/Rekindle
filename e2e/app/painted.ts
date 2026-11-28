// Whether a part is seen, by the owner's rule: hiding it changes the pixels
// where it sits. A box in the DOM is not proof (a part can be laid out yet
// unpainted, covered, or fully transparent over a transparent window), and
// neither is elementFromPoint. Two screenshots of the window `b` is on, one
// with the part as drawn and one with it `visibility: hidden`, compared
// inside the part's box. The part is restored before this returns.

import type { Browser } from "webdriverio";
import { decodePng } from "./png.ts";

export interface Painted {
  /** Pixels inside the part's box that differ once it is hidden. */
  changed: number;
  /** Pixels inside the part's box, in screenshot pixels. */
  total: number;
  /** The part's box in CSS pixels: left, top, right, bottom. */
  box: number[];
}

/** How much of `selector`'s first match shows: the pixels that change inside its box when it is hidden. */
export async function painted(b: Browser, selector: string): Promise<Painted> {
  const box = (await b.execute((sel: string) => {
    const el = document.querySelector<HTMLElement>(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return [r.left, r.top, r.right, r.bottom, window.innerWidth];
  }, selector)) as number[] | null;
  if (!box) throw new Error(`painted: no ${selector}`);
  const shown = decodePng(Buffer.from(await b.takeScreenshot(), "base64"));
  await b.execute((sel: string) => {
    document.querySelector<HTMLElement>(sel)?.style.setProperty("visibility", "hidden");
  }, selector);
  let hidden: ReturnType<typeof decodePng>;
  try {
    hidden = decodePng(Buffer.from(await b.takeScreenshot(), "base64"));
  } finally {
    await b.execute((sel: string) => {
      document.querySelector<HTMLElement>(sel)?.style.removeProperty("visibility");
    }, selector);
  }
  const scale = shown.width / box[4];
  const [x0, y0] = [
    Math.max(0, Math.floor(box[0] * scale)),
    Math.max(0, Math.floor(box[1] * scale)),
  ];
  const [x1, y1] = [
    Math.min(shown.width, Math.ceil(box[2] * scale)),
    Math.min(shown.height, Math.ceil(box[3] * scale)),
  ];
  let changed = 0;
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const i = (y * shown.width + x) * 4;
      if (
        shown.data[i] !== hidden.data[i] ||
        shown.data[i + 1] !== hidden.data[i + 1] ||
        shown.data[i + 2] !== hidden.data[i + 2] ||
        shown.data[i + 3] !== hidden.data[i + 3]
      ) {
        changed += 1;
      }
    }
  }
  return {
    changed,
    total: Math.max(0, x1 - x0) * Math.max(0, y1 - y0),
    box: box.slice(0, 4).map(Math.round),
  };
}
