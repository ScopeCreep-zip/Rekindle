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

/**
 * The current window's skinned frame (lead, SKW1): every drawn node of the
 * frame's roles (title bar, window buttons, menu bar and titles, the chat
 * title), as `id role art` lines sorted, art being the node's own skin://
 * files; and whether a default (unskinned) title bar is drawn instead.
 */
export async function frameNodes(): Promise<{ nodes: string[]; defaultTitlebar: boolean }> {
  return (await browser.execute(() => {
    const drawn = (e: Element): boolean => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
    const frameRole = /^(titleBar|windowButton|menuBar|menuTitle|chatTitle)(:|$)/;
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]"))
      .filter((n) => frameRole.test(n.dataset.skinRole ?? "") && drawn(n))
      .map((n) => {
        const cs = getComputedStyle(n);
        const art = [cs.backgroundImage, cs.borderImageSource]
          .flatMap((v) => v.match(/skin:\/\/[^)"]+/g) ?? [])
          .map((u) => u.split("/").slice(-1)[0])
          .join(",");
        return `${n.dataset.skinNode} ${n.dataset.skinRole} ${art || "-"}`;
      })
      .sort();
    const tb = document.querySelector(".xfire-titlebar");
    return { nodes, defaultTitlebar: !!tb && drawn(tb) };
  })) as { nodes: string[]; defaultTitlebar: boolean };
}

/**
 * Lead (SKW1 parity): everything the tree exposes on the nodes `ids` in the
 * current window, one line each: every `data-*` and `aria-*` attribute, the
 * class, the box and the full art URLs (background and border image), or
 * "absent" when the window draws no such node.
 */
export async function frameNodeDetail(ids: string[]): Promise<string[]> {
  return (await browser.execute((want: string[]) =>
    want.map((id) => {
      const n = document.querySelector<HTMLElement>(`.skin-tree [data-skin-node="${id}"]`);
      if (!n) return `${id}: absent`;
      const attrs = Array.from(n.attributes)
        .filter((a) => a.name.startsWith("data-") || a.name.startsWith("aria-") || a.name === "class" || a.name === "title")
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`)
        .join(" ");
      const r = n.getBoundingClientRect();
      const cs = getComputedStyle(n);
      const art = [cs.backgroundImage, cs.borderImageSource].flatMap((v) => v.match(/skin:\/\/[^)"]+/g) ?? []).join(" , ");
      return `${id}: ${attrs} box=${[r.x, r.y, r.width, r.height].map(Math.round).join(",")} art=${art || "-"}`;
    }),
  ids)) as string[];
}
