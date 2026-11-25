// What a skin paints in the current window, read from computed styles (the
// skin sheet is cross-origin, so its rules cannot be read): every element
// drawn with skin:// art, with its classes, box, art and ancestors.

import { browser } from "@wdio/globals";

/** Every element in the current window drawn with skin:// art, one line each. */
export async function artElements(): Promise<string[]> {
  return (await browser.execute(() => {
    const name = (el: Element): string => `${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}${el.classList.length ? "." + Array.from(el.classList).join(".") : ""}`;
    const out: string[] = [];
    for (const el of Array.from(document.querySelectorAll<HTMLElement>("body *"))) {
      const cs = getComputedStyle(el);
      const art = [cs.backgroundImage, cs.borderImageSource].filter((v) => v && v.includes("skin://"));
      if (art.length === 0) continue;
      const path: string[] = [];
      for (let p = el.parentElement; p && p !== document.body && path.length < 5; p = p.parentElement) path.push(name(p));
      const r = el.getBoundingClientRect();
      const file = art.map((a) => (a.match(/skin:\/\/[^)"]+/g) ?? []).map((u) => u.split("/").slice(-2).join("/")).join(",")).join(" ");
      out.push(`${name(el)} [${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}] art=${file.slice(0, 120)} in ${path.reverse().join(" > ")}`);
    }
    return out;
  })) as string[];
}

/** The current window's kind (plan SB1: `data-window` on its document root, `<html>`). */
export async function windowKind(): Promise<string> {
  return (await browser.execute(() => document.documentElement.dataset.window ?? "")) as string;
}
