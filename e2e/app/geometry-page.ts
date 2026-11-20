// Plan E7.14 S4 (moved from E7.12's e2e/fixtures/layout.ts and
// layout-text.ts): the geometry checks, as page-side functions. Each runs
// inside the window under test with its root element and one argument, and
// returns what it found wrong (an empty list or the expected object when
// nothing is). They cross into the page as source text (geometry.ts), so
// every function here is self-contained: no closures, no imports, only the
// `window.__geo` helpers `geometryLib` installs.

/** Overlays that are meant to sit on top of other content. */
export const OVERLAYS = [
  ".skip-link",
  "[role=dialog]",
  "[role=alertdialog]",
  "[role=tooltip]",
  "[role=menu]",
  "[role=listbox]",
  // Toasts and screen-reader announcers. A `role=log` (the message lists,
  // `LiveRegion`) is live too, but it is content and is checked.
  "[aria-live]:not([role=log])",
  ".sr-only",
];

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Clash {
  a: string;
  b: string;
  boxA: Box;
  boxB: Box;
}

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
 * The page-side helpers, installed once per page as `window.__geo`. A
 * function's source crosses the boundary; closures do not.
 */
export function geometryLib(): void {
  const w = window as any;
  if (w.__geo) return;
  const describe = (el: Element): string => {
    const cls = typeof el.className === "string" && el.className.trim()
      ? "." + el.className.trim().split(/\s+/).slice(0, 3).join(".")
      : "";
    const text = (el.textContent ?? "").trim().slice(0, 24);
    return `${el.tagName.toLowerCase()}${cls}${text ? ` "${text}"` : ""}`;
  };
  const shown = (el: Element): boolean => {
    // Also false under a closed <details> or other content-visibility: hidden.
    if (!el.checkVisibility({ visibilityProperty: true, opacityProperty: true })) return false;
    const s = getComputedStyle(el);
    if (s.display === "none" || s.visibility !== "visible" || Number(s.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const clips = (s: CSSStyleDeclaration) =>
    s.overflowX !== "visible" || s.overflowY !== "visible";
  /** The part of `r` left after every clipping ancestor (and `self`) and the viewport. */
  const visibleRect = (el: Element, r: DOMRect | Box): Box | null => {
    let x0 = "x" in r ? r.x : 0, y0 = r.y, x1 = x0 + ("w" in r ? r.w : r.width), y1 = y0 + ("h" in r ? r.h : r.height);
    for (let a: Element | null = el; a; a = a.parentElement) {
      const s = getComputedStyle(a);
      if (s.position === "fixed" && a !== el) break;
      if (clips(s)) {
        const c = a.getBoundingClientRect();
        x0 = Math.max(x0, c.left); y0 = Math.max(y0, c.top);
        x1 = Math.min(x1, c.right); y1 = Math.min(y1, c.bottom);
      }
    }
    x0 = Math.max(x0, 0); y0 = Math.max(y0, 0);
    x1 = Math.min(x1, window.innerWidth); y1 = Math.min(y1, window.innerHeight);
    return x1 - x0 > 0.5 && y1 - y0 > 0.5 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
  };
  const INTERACTIVE = "button, a[href], input, select, textarea, [role=button], [role=tab], [role=checkbox], [role=switch], [tabindex]:not([tabindex='-1'])";
  /** The glyph box of an element's own text, or null when it has none. */
  const textBox = (el: Element): Box | null => {
    let box: Box | null = null;
    for (const n of Array.from(el.childNodes)) {
      if (n.nodeType !== Node.TEXT_NODE || !(n.textContent ?? "").trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      for (const r of Array.from(range.getClientRects())) {
        if (r.width < 0.5 || r.height < 0.5) continue;
        box = box
          ? ((): Box => {
              const x = Math.min(box!.x, r.left), y = Math.min(box!.y, r.top);
              return { x, y, w: Math.max(box!.x + box!.w, r.right) - x, h: Math.max(box!.y + box!.h, r.bottom) - y };
            })()
          : { x: r.left, y: r.top, w: r.width, h: r.height };
      }
    }
    return box;
  };
  w.__geo = {
    describe,
    shown,
    visibleRect,
    /** Visible interactive and text elements under `root`, outside the overlays. */
    candidates(root: Element, skip: string[]): { el: Element; box: Box }[] {
      const out: { el: Element; box: Box }[] = [];
      const skipSel = skip.join(", ");
      for (const el of Array.from(root.querySelectorAll("*"))) {
        if (skipSel && el.closest(skipSel)) continue;
        if (!shown(el)) continue;
        const own = el.matches(INTERACTIVE) ? el.getBoundingClientRect() : textBox(el);
        if (!own) continue;
        const box = visibleRect(el, own);
        if (box) out.push({ el, box });
      }
      return out;
    },
    INTERACTIVE,
  };
}

/**
 * No two visible interactive or text elements under `root` intersect by
 * more than a pixel each way, unless one contains the other or one sits in
 * a declared overlay (`skip`: `OVERLAYS` plus a case's own).
 */
export function overlaps(rootEl: Element, skip: string[]): Clash[] {
  const g = (window as any).__geo;
  const items = g.candidates(rootEl, skip) as { el: Element; box: Box }[];
  const out: Clash[] = [];
  for (let i = 0; i < items.length; i++) {
    for (let j = i + 1; j < items.length; j++) {
      const a = items[i], b = items[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const ix = Math.min(a.box.x + a.box.w, b.box.x + b.box.w) - Math.max(a.box.x, b.box.x);
      const iy = Math.min(a.box.y + a.box.h, b.box.y + b.box.h) - Math.max(a.box.y, b.box.y);
      if (ix > 1 && iy > 1) out.push({ a: g.describe(a.el), b: g.describe(b.el), boxA: a.box, boxB: b.box });
    }
  }
  return out.slice(0, 12);
}

/** Neither the document nor `root` scrolls sideways (both 0 when fine). */
export function horizontalOverflow(rootEl: Element): { document: number; root: number } {
  const doc = document.scrollingElement!;
  return {
    document: doc.scrollWidth - doc.clientWidth,
    root: rootEl.scrollWidth - rootEl.clientWidth,
  };
}

/**
 * Every visible block-level text element fits its box or shows the declared
 * ellipsis. Form fields and declared scrollers (`overflow-x: auto|scroll`)
 * scroll by design.
 */
export function textOverflows(rootEl: Element, skip: string[]): { el: string; scrollWidth: number; clientWidth: number }[] {
  const g = (window as any).__geo;
  const out: { el: string; scrollWidth: number; clientWidth: number }[] = [];
  for (const { el } of g.candidates(rootEl, skip) as { el: HTMLElement }[]) {
    if (el.matches("input, textarea, select")) continue;
    const s = getComputedStyle(el);
    if (s.display.startsWith("inline") && s.display !== "inline-block" && s.display !== "inline-flex") continue;
    if (s.overflowX === "auto" || s.overflowX === "scroll") continue;
    if (el.scrollWidth <= el.clientWidth + 1) continue;
    if (s.textOverflow === "ellipsis" && s.overflowX !== "visible") continue;
    out.push({ el: g.describe(el), scrollWidth: el.scrollWidth, clientWidth: el.clientWidth });
  }
  return out.slice(0, 12);
}

export interface FlexContract {
  display: string;
  direction: string;
  fillsWindow: boolean;
  growing: number;
  footerAtBottom: boolean;
}

/** The contract every window root keeps (Preserve Flex Layout). */
export const FLEX_CONTRACT: FlexContract = {
  display: "flex",
  direction: "column",
  fillsWindow: true,
  growing: 1,
  footerAtBottom: true,
};

/**
 * The window root as a flex container: a full-height column with exactly
 * one growing child, its last in-flow child ending at the window's bottom
 * edge (`FLEX_CONTRACT` when kept).
 */
export function flexContract(el: Element): FlexContract {
  const s = getComputedStyle(el);
  const r = el.getBoundingClientRect();
  const flow = Array.from(el.children).filter((c) => {
    const cs = getComputedStyle(c);
    return cs.display !== "none" && cs.position !== "absolute" && cs.position !== "fixed" &&
      c.getBoundingClientRect().height > 0;
  });
  const last = flow[flow.length - 1];
  return {
    display: s.display,
    direction: s.flexDirection,
    fillsWindow: Math.abs(r.height - window.innerHeight) <= 1,
    growing: flow.filter((c) => Number(getComputedStyle(c).flexGrow) > 0).length,
    footerAtBottom: last ? Math.abs(last.getBoundingClientRect().bottom - r.bottom) <= 1 : false,
  };
}

/**
 * Every focusable element under `root` takes focus, looks different in its
 * focused state (outline, shadow, border, background or text colour), and
 * a ring it draws (outline or box-shadow) is not cut by a clipping
 * ancestor.
 *
 * The focused state is forced, as Web Inspector's "force state" does:
 * WebKit matches `:focus`/`:focus-visible` only while the document has
 * system focus, and a test app's window is usually not frontmost on a
 * desktop in use (probe 2026-11-19: `document.hasFocus()` false,
 * `visibilityState` hidden, `activeElement` moved, `:focus` false), nor can
 * the suite take focus (macOS and Wayland refuse focus stealing), and the
 * driver's keys are untrusted events, so they set no keyboard modality.
 * Each rule that names `:focus-visible`, `:focus` or `:focus-within` gets a
 * clone right after it with the pseudo-class swapped for a data attribute,
 * so the cascade, specificity and order stay WebKit's own; the attribute is
 * set on the element (and on it and its ancestors for `:focus-within`)
 * while its computed style is read; the clones are removed at the end. The
 * app draws its own ring for every element (`*:focus-visible`,
 * src/styles/regions.toml), so no UA rule is needed.
 */
export function focusIndicators(rootEl: Element, skip: string[]): { el: string; why: string }[] {
  const g = (window as any).__geo;
  const skipSel = skip.join(", ");
  const out: { el: string; why: string }[] = [];
  const FOCUS = "data-geo-focus";
  const WITHIN = "data-geo-focus-within";
  const swap = (sel: string): string =>
    sel.replace(/:focus-visible\b/g, `[${FOCUS}]`).replace(/:focus-within\b/g, `[${WITHIN}]`).replace(/:focus(?![-\w])/g, `[${FOCUS}]`);
  type Owner = { cssRules: CSSRuleList; insertRule(rule: string, index: number): number; deleteRule(index: number): void };
  const clones: { owner: Owner; rule: CSSRule }[] = [];
  const isOwner = (r: unknown): r is Owner =>
    !!r && typeof (r as Owner).insertRule === "function" && (r as Owner).cssRules !== undefined;
  const clone = (owner: Owner): void => {
    for (let i = 0; i < owner.cssRules.length; i++) {
      const r = owner.cssRules[i];
      if (r instanceof CSSStyleRule && /:focus/.test(r.selectorText)) {
        owner.insertRule(swap(r.selectorText) + r.cssText.slice(r.selectorText.length), i + 1);
        clones.push({ owner, rule: owner.cssRules[i + 1] });
        i++;
        continue;
      }
      // @media, @supports, @layer, @container blocks and nested style rules.
      if (isOwner(r)) clone(r);
    }
  };
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList | null = null;
    try { rules = sheet.cssRules; } catch { rules = null; }
    if (rules) clone(sheet);
  }
  const look = (el: HTMLElement): string => {
    const s = getComputedStyle(el);
    return [s.outlineStyle, s.outlineWidth, s.outlineColor, s.boxShadow, s.backgroundColor,
      s.borderTopColor, s.borderRightColor, s.borderBottomColor, s.borderLeftColor, s.color].join("|");
  };
  const force = (el: HTMLElement, on: boolean): void => {
    el.toggleAttribute(FOCUS, on);
    for (let a: Element | null = el; a; a = a.parentElement) a.toggleAttribute(WITHIN, on);
  };
  try {
    const els = Array.from(rootEl.querySelectorAll(g.INTERACTIVE)) as HTMLElement[];
    for (const el of els) {
      if (skipSel && el.closest(skipSel)) continue;
      if (!g.shown(el) || (el as HTMLButtonElement).disabled) continue;
      el.scrollIntoView({ block: "nearest", inline: "nearest" });
      // Read before focusing, as E7.12's check did: a focus handler may
      // restyle the element itself (Kobalte highlights a menubar trigger on
      // focus), and that change is an indicator too.
      const before = look(el);
      el.focus();
      if (document.activeElement !== el) { out.push({ el: g.describe(el), why: "not focusable" }); continue; }
      force(el, true);
      const s = getComputedStyle(el);
      const after = look(el);
      const ow = s.outlineStyle === "none" ? 0 : parseFloat(s.outlineWidth) || 0;
      const shadow = s.boxShadow;
      const offset = parseFloat(s.outlineOffset) || 0;
      const r = el.getBoundingClientRect();
      force(el, false);
      if (after === before) { out.push({ el: g.describe(el), why: `no indicator${el.matches(":hover") ? " (hovered)" : ""}` }); continue; }
      if (ow === 0 && shadow === "none") continue;
      const grow = Math.max(0, ow + offset);
      const ring = { x: r.left - grow, y: r.top - grow, w: r.width + 2 * grow, h: r.height + 2 * grow };
      const seen = g.visibleRect(el.parentElement ?? el, ring);
      if (!seen || seen.w + 0.5 < ring.w || seen.h + 0.5 < ring.h) {
        out.push({ el: g.describe(el), why: `ring clipped to ${seen ? `${seen.w.toFixed(1)}x${seen.h.toFixed(1)}` : "nothing"} of ${ring.w.toFixed(1)}x${ring.h.toFixed(1)}` });
      }
    }
  } finally {
    (document.activeElement as HTMLElement | null)?.blur();
    for (const { owner, rule } of clones.reverse()) {
      const i = Array.from(owner.cssRules).indexOf(rule);
      if (i >= 0) owner.deleteRule(i);
    }
  }
  return out.slice(0, 12);
}

/**
 * The gaps the stylesheet declares on `containers` (computed `row-gap`,
 * `column-gap`, and each child's non-`auto` margins) sit on the spacing
 * grid: whole multiples of `--layout-grid` (`src/styles/global.css`). The
 * step is read from the page, never written here.
 */
export function spacingOffScale(rootEl: Element, containers: string[]): string[] {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--layout-grid").trim();
  if (!raw) return ["--layout-grid is not defined"];
  const probe = document.createElement("div");
  probe.style.position = "absolute";
  probe.style.width = raw;
  document.body.appendChild(probe);
  const step = probe.getBoundingClientRect().width;
  probe.remove();
  if (!(step > 0)) return [`--layout-grid is not a length ("${raw}")`];
  const onScale = (px: number) => Math.abs(px / step - Math.round(px / step)) < 0.01;
  const out: string[] = [];
  for (const sel of containers) {
    const found = Array.from(rootEl.querySelectorAll(sel));
    // A selector that matches nothing would pass vacuously.
    if (found.length === 0) out.push(`${sel} matches nothing`);
    for (const c of found) {
      const s = getComputedStyle(c);
      for (const [p, v] of [["row-gap", s.rowGap], ["column-gap", s.columnGap]]) {
        if (v !== "normal" && !onScale(parseFloat(v))) out.push(`${sel} ${p} ${v}`);
      }
      for (const child of Array.from(c.children)) {
        // The computed (not resolved) value keeps `auto`, which
        // distributes free space and is not a gap.
        const map = child.computedStyleMap();
        for (const p of ["margin-top", "margin-right", "margin-bottom", "margin-left"]) {
          const v = map.get(p);
          if (!(v instanceof CSSUnitValue)) continue;
          if (!onScale(v.to("px").value)) out.push(`${sel} > ${child.tagName.toLowerCase()}.${child.className} ${p} ${v}`);
        }
      }
    }
  }
  return out.slice(0, 12);
}

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
