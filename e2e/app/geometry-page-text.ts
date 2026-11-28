// Plan E7.14 S4 (moved from E7.12's e2e/fixtures/layout-text.ts): the text,
// row and target checks, as page-side functions, split from
// geometry-page.ts by size. The same rules hold: each runs inside the
// window under test with its root element and one argument, returns what it
// found wrong, and crosses into the page as source text (geometry.ts), so
// every function here is self-contained, using only the `window.__geo`
// helpers geometry-page.ts's `geometryLib` installs.

/**
 * The fewest letters or digits an ellipsized label may still show. Below
 * four it identifies nothing: the identity bar's key cut to "aa…" (two
 * base58 characters, 58² ≈ 3.4 k values, shared by every key that starts
 * the same) and two-letter stubs such as "Me…" or "Fr…" match many rows.
 * An ellipsis costs about one character's width, so a box too narrow for
 * four characters is spending a fifth of itself on the ellipsis.
 */
export const MIN_ELLIPSIS_CHARS = 4;

/**
 * The share of the titlebar's content width the title may take (the rule
 * `.xfire-titlebar-text { max-width: 50% }`): a long name ellipsizes in the
 * left half, like the in-pane headers, instead of running into the window
 * controls.
 */
export const TITLE_SHARE = 0.5;

/** How far apart (CSS px) the composer's controls' vertical centres may sit. */
export const CENTRE_TOLERANCE = 2;

/** WCAG 2.2 SC 2.5.8's minimum target, in CSS px. */
export const MIN_TARGET = 24;

/** The narrowest a text field may render and still be typed into. */
export const MIN_FIELD_WIDTH = 80;

/**
 * No visible text under `root` is cut sideways by an ancestor (or itself)
 * that clips (`overflow` hidden, clip, auto or scroll), unless an element
 * between the text and that clipper ellipsizes (`text-overflow: ellipsis`,
 * `overflow: hidden`, `white-space: nowrap`) and is itself wholly inside
 * the clipper, so the reader sees the ellipsis. Boxes are compared
 * unclipped: the clipping is what this looks for.
 */
export function textClipped(rootEl: Element, skip: string[]): string[] {
  const g = (window as any).__geo;
  const skipSel = skip.join(", ");
  const out: string[] = [];
  const ellipsizes = (s: CSSStyleDeclaration) =>
    s.textOverflow === "ellipsis" && s.overflowX === "hidden" && s.whiteSpace.startsWith("nowrap");
  const padX = (a: Element): [number, number] => {
    const r = a.getBoundingClientRect();
    const left = r.left + a.clientLeft;
    return [left, left + a.clientWidth];
  };
  for (const el of Array.from(rootEl.querySelectorAll("*"))) {
    if (skipSel && el.closest(skipSel)) continue;
    if (el.closest("input, textarea, select, option") || !g.shown(el)) continue;
    let x0 = Infinity, x1 = -Infinity;
    for (const n of Array.from(el.childNodes)) {
      if (n.nodeType !== Node.TEXT_NODE || !(n.textContent ?? "").trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      for (const r of Array.from(range.getClientRects())) {
        if (r.width < 0.5 || r.height < 0.5) continue;
        x0 = Math.min(x0, r.left); x1 = Math.max(x1, r.right);
      }
    }
    if (!(x1 > x0)) continue;
    let excused: Element | null = null;
    for (let a: Element | null = el; a && a !== document.documentElement; a = a.parentElement) {
      const s = getComputedStyle(a);
      if (s.position === "fixed" && a !== el) break;
      if (!excused && ellipsizes(s)) excused = a;
      if (s.overflowX === "visible") continue;
      const [c0, c1] = padX(a);
      if (x0 >= c0 - 1 && x1 <= c1 + 1) continue;
      if (excused && excused !== a) {
        const [e0, e1] = padX(excused);
        if (e0 >= c0 - 1 && e1 <= c1 + 1) continue;
      } else if (excused === a) continue;
      out.push(`${g.describe(el)} text [${Math.round(x0)}, ${Math.round(x1)}] cut by ${g.describe(a).slice(0, 40)} [${Math.round(c0)}, ${Math.round(c1)}]`);
      x0 = -Infinity;
      break;
    }
    // The window edge clips too.
    if (x0 === -Infinity || (x0 >= -1 && x1 <= window.innerWidth + 1)) continue;
    if (excused) {
      const [e0, e1] = padX(excused);
      if (e0 >= -1 && e1 <= window.innerWidth + 1) continue;
    }
    out.push(`${g.describe(el)} text [${Math.round(x0)}, ${Math.round(x1)}] cut by the window [0, ${window.innerWidth}]`);
  }
  return out.slice(0, 12);
}

/** The content panes under `root` that render (`panes`) never scroll sideways. */
export function panesScrollingSideways(rootEl: Element, panes: string[]): string[] {
  const out: string[] = [];
  for (const sel of panes) {
    const found = Array.from(rootEl.querySelectorAll(sel));
    // A selector that matches nothing would pass vacuously.
    if (found.length === 0) out.push(`${sel} matches nothing`);
    for (const p of found) {
      if (p.scrollWidth > p.clientWidth + 1) out.push(`${sel} scrollWidth ${p.scrollWidth} > clientWidth ${p.clientWidth}`);
    }
  }
  return out;
}

/**
 * Badges and counts sit wholly inside their row: never squeezed below
 * their content, not cut sideways by a clipping ancestor or the window,
 * nor spilling out of their row.
 */
export function badgesOutOfRow(rootEl: Element): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  for (const el of Array.from(rootEl.querySelectorAll("[class*='badge'], [class*='-count']"))) {
    if (!g.shown(el)) continue;
    const r = el.getBoundingClientRect();
    // A count is never truncated, not even to an ellipsis.
    if (el.scrollWidth > el.clientWidth + 1) out.push(`${g.describe(el)} squeezed to ${el.clientWidth}px of ${el.scrollWidth}px`);
    for (let a = el.parentElement; a && a !== document.documentElement; a = a.parentElement) {
      const s = getComputedStyle(a);
      if (s.position === "fixed") break;
      if (s.overflowX === "visible" && s.overflowY === "visible") continue;
      const c = a.getBoundingClientRect();
      const left = c.left + a.clientLeft, right = left + a.clientWidth;
      if (r.left < left - 0.5 || r.right > right + 0.5) {
        out.push(`${g.describe(el)} [${r.left.toFixed(1)}, ${r.right.toFixed(1)}] cut by ${g.describe(a).slice(0, 40)} [${left.toFixed(1)}, ${right.toFixed(1)}]`);
        break;
      }
    }
    if (r.left < -0.5 || r.right > window.innerWidth + 0.5) out.push(`${g.describe(el)} [${r.left.toFixed(1)}, ${r.right.toFixed(1)}] cut by the window`);
    const row = el.parentElement!.getBoundingClientRect();
    if (r.top < row.top - 0.5 || r.bottom > row.bottom + 0.5) out.push(`${g.describe(el)} spills out of its row vertically`);
  }
  return out.slice(0, 12);
}

/** No visible text or accessible label under `root` shows a missing value. */
export function placeholderText(rootEl: Element): string[] {
  const g = (window as any).__geo;
  const re = /\b(undefined|null|NaN)\b|\[object Object\]/;
  const out: string[] = [];
  const walker = document.createTreeWalker(rootEl, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const p = n.parentElement;
    if (p && g.shown(p) && re.test(n.textContent ?? "")) out.push(`${g.describe(p)}: "${(n.textContent ?? "").trim().slice(0, 40)}"`);
  }
  for (const el of Array.from(rootEl.querySelectorAll("[aria-label], [title], [placeholder]"))) {
    for (const attr of ["aria-label", "title", "placeholder"]) {
      const v = el.getAttribute(attr);
      if (v && re.test(v)) out.push(`${g.describe(el)} ${attr}="${v.slice(0, 40)}"`);
    }
  }
  return out.slice(0, 12);
}

/**
 * A timestamp is one line and shares that line with the element before it
 * (a message header's sender name), never wrapping under it.
 */
export function timestampsOffLine(rootEl: Element): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  for (const el of Array.from(rootEl.querySelectorAll("[class*='timestamp']"))) {
    if (!g.shown(el)) continue;
    const range = document.createRange();
    range.selectNodeContents(el);
    const tops = new Set(Array.from(range.getClientRects()).filter((r) => r.width > 0.5).map((r) => Math.round(r.top)));
    if (tops.size > 1) out.push(`${g.describe(el)} wraps over ${tops.size} lines`);
    let prev = el.previousElementSibling;
    while (prev && !g.shown(prev)) prev = prev.previousElementSibling;
    if (!prev) continue;
    const a = el.getBoundingClientRect(), b = prev.getBoundingClientRect();
    if (Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) < 1) out.push(`${g.describe(el)} sits on its own line under ${g.describe(prev)}`);
  }
  return out.slice(0, 12);
}

/** Every visible text field under `root` is at least `min` wide, never squeezed by its row. */
export function fieldsSqueezed(rootEl: Element, min: number): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  const sel = "input:not([type]), input[type=text], input[type=search], input[type=password], input[type=email], input[type=url], textarea, select";
  for (const el of Array.from(rootEl.querySelectorAll(sel))) {
    if (!g.shown(el)) continue;
    const w = el.getBoundingClientRect().width;
    if (w < min) out.push(`${g.describe(el)}${el.id ? `#${el.id}` : ""} is ${w.toFixed(1)}px wide`);
  }
  return out.slice(0, 12);
}

/**
 * Every ellipsized element under `root` that does cut its text still shows
 * at least `min` letters or digits (fewer only when its whole text has
 * fewer).
 */
export function ellipsisIllegible(rootEl: Element, [skip, min]: [string[], number]): string[] {
  const g = (window as any).__geo;
  const skipSel = skip.join(", ");
  const meaningful = /[\p{L}\p{N}]/u;
  const ctx = document.createElement("canvas").getContext("2d")!;
  const out: string[] = [];
  for (const el of Array.from(rootEl.querySelectorAll("*")) as HTMLElement[]) {
    if (skipSel && el.closest(skipSel)) continue;
    if (el.closest("input, textarea, select, option") || !g.shown(el)) continue;
    const s = getComputedStyle(el);
    if (s.textOverflow !== "ellipsis" || s.overflowX !== "hidden" || !s.whiteSpace.startsWith("nowrap")) continue;
    if (el.scrollWidth <= el.clientWidth + 1) continue;
    const r = el.getBoundingClientRect();
    ctx.font = s.font;
    const edge = r.left + el.clientLeft + el.clientWidth - parseFloat(s.paddingRight) - ctx.measureText("…").width;
    let shown = 0, total = 0;
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const text = n.textContent ?? "";
      for (let i = 0; i < text.length; i++) {
        if (!meaningful.test(text[i])) continue;
        total++;
        const range = document.createRange();
        range.setStart(n, i);
        range.setEnd(n, i + 1);
        const c = range.getBoundingClientRect();
        if (c.width > 0 && c.right <= edge + 0.5) shown++;
      }
    }
    if (shown < Math.min(min, total)) {
      out.push(`${g.describe(el)} shows ${shown} of ${total} letters/digits in ${el.clientWidth}px`);
    }
  }
  return out.slice(0, 12);
}

/**
 * Each window's titlebar title ends within `share` of the bar and, when
 * its text is longer than that box, is cut there with an ellipsis.
 */
export function titlebarTitleUnbounded(rootEl: Element, share: number): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  for (const bar of Array.from(rootEl.querySelectorAll(".xfire-titlebar"))) {
    const title = bar.querySelector(".xfire-titlebar-text") as HTMLElement | null;
    if (!title || !g.shown(title)) {
      out.push(`${g.describe(bar)} has no visible title`);
      continue;
    }
    const b = bar.getBoundingClientRect(), bs = getComputedStyle(bar);
    const left = b.left + parseFloat(bs.paddingLeft);
    const limit = left + (b.width - parseFloat(bs.paddingLeft) - parseFloat(bs.paddingRight)) * share;
    const t = title.getBoundingClientRect();
    if (t.right > limit + 1) out.push(`${g.describe(title)} ends at ${t.right.toFixed(1)}, past ${limit.toFixed(1)}`);
    const s = getComputedStyle(title);
    const cut = title.scrollWidth > title.clientWidth + 1;
    const ellipsizes = s.textOverflow === "ellipsis" && s.overflowX === "hidden" && s.whiteSpace.startsWith("nowrap");
    if (cut && !ellipsizes) out.push(`${g.describe(title)} is cut without an ellipsis`);
  }
  return out;
}

/** The composer's in-flow controls (buttons and the text field) share one centre line, within `tol` px. */
export function composerMisaligned(rootEl: Element, tol: number): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  for (const shell of Array.from(rootEl.querySelectorAll(".message-input-shell"))) {
    const items = (Array.from(shell.children) as HTMLElement[]).filter((el) => {
      if (!g.shown(el) || !el.matches("button, textarea, input")) return false;
      const pos = getComputedStyle(el).position;
      return pos !== "absolute" && pos !== "fixed";
    });
    const centres = items.map((el) => {
      const r = el.getBoundingClientRect();
      return { el, c: r.top + r.height / 2 };
    });
    if (centres.length < 2) continue;
    const cs = centres.map((x) => x.c);
    if (Math.max(...cs) - Math.min(...cs) > tol) {
      out.push(centres.map((x) => `${g.describe(x.el).slice(0, 32)} @${x.c.toFixed(1)}`).join(" | "));
    }
  }
  return out;
}

/**
 * Plan E7.3 "Target size": every titlebar button is at least `min` × `min`
 * CSS px. They sit side by side, so SC 2.5.8's spacing exception does not
 * apply.
 */
export function titlebarTargetsSmall(rootEl: Element, min: number): string[] {
  const g = (window as any).__geo;
  const out: string[] = [];
  for (const btn of Array.from(rootEl.querySelectorAll(".xfire-titlebar-btn"))) {
    if (!g.shown(btn)) continue;
    const r = btn.getBoundingClientRect();
    if (r.width < min - 0.01 || r.height < min - 0.01) {
      out.push(`${g.describe(btn)} is ${r.width.toFixed(1)}x${r.height.toFixed(1)}`);
    }
  }
  return out;
}
