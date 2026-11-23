// Plan E7.14 S4 (moved from E7.12's e2e/fixtures/layout.ts): the box,
// overflow, flex, focus and spacing checks, as page-side functions (the
// text, row and target checks are in geometry-page-text.ts). Each runs
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
    // Text that does not fit needs text: a box holding none overflows only
    // by what the skin draws past it on purpose (a served ::before hit area
    // growing an 18 px button to 24 px, WCAG 2.5.8; a frame's ::after art).
    if ((el.textContent ?? "").trim() === "") continue;
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
 * one growing child, its last in-flow child ending at the bottom of the
 * root's content box (`FLEX_CONTRACT` when kept).
 *
 * The content box, not the border box: every root is `.app-frame`, whose
 * border box is the skin's window frame (plan E7.7.7 `window.frame`,
 * `src/styles/generated/regions/window.css`: `border-width:
 * var(--skin-region-window-frame-border, 1px)`), as wide as the frame's
 * nine-slice edges (Symbiosis 13 px on every side) and
 * 1 px for the token look. The frame insets every child by that edge, the
 * footer included, so a footer held down ends `border-bottom-width` above
 * the root's outer bottom (measured 2026-11-20: 437 vs 450, 537 vs 550 on
 * Symbiosis's 13 px edge). A frame laid as padding instead (the chat
 * window's pack frame under the app titlebar, SK3/SK5: 0 border, 13 px
 * padding, r24 DM at 720x825) insets it the same way, so the foot is
 * measured against the content box: border and padding off. A footer that
 * stops short of the frame's inner edge (a margin, a wrapper that breaks
 * the flex chain) still fails.
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
    footerAtBottom: last
      ? Math.abs(last.getBoundingClientRect().bottom - (r.bottom - parseFloat(s.borderBottomWidth) - parseFloat(s.paddingBottom))) <= 1
      : false,
  };
}

/**
 * Where the window root's foot is, when the flex contract fails: its last
 * in-flow child and that child's bottom against the root's bottom, border
 * and padding, so the finding says by how much and against what.
 */
export function footerGap(el: Element): { last: string; lastBottom: number; rootBottom: number; borderBottom: number; paddingBottom: number } | null {
  const s = getComputedStyle(el);
  const flow = Array.from(el.children).filter((c) => {
    const cs = getComputedStyle(c);
    return cs.display !== "none" && cs.position !== "absolute" && cs.position !== "fixed" && c.getBoundingClientRect().height > 0;
  });
  const last = flow[flow.length - 1];
  if (!last) return null;
  return {
    last: `${last.tagName.toLowerCase()}.${(last.className || "").toString().split(" ").join(".")}`,
    lastBottom: Math.round(last.getBoundingClientRect().bottom * 10) / 10,
    rootBottom: Math.round(el.getBoundingClientRect().bottom * 10) / 10,
    borderBottom: parseFloat(s.borderBottomWidth),
    paddingBottom: parseFloat(s.paddingBottom),
  };
}

/** How a skin's tree sits in its window (plan SK4), at one size. */
export interface TreeFit {
  /** The tree's box lies inside the viewport. */
  inWindow: boolean;
  /** The tree's box and the viewport, px, so a finding says where it sticks out. */
  box: { left: number; top: number; right: number; bottom: number; viewport: [number, number] };
  /** The window's width and height beyond the tree's (the frame's outsets). */
  gapWidth: number;
  gapHeight: number;
  /** The window's height beyond the views node's (its content area). */
  viewsGapHeight: number | null;
}

/**
 * A tree-laid window's stand-in for the flex contract (plan SK4): the
 * pack places every node itself, so there is no flex chain to keep. What
 * the contract promised, content that fills the window and a foot held
 * down as it grows, holds when the tree lies inside the window and the
 * tree and its views node grow with it: the gaps stay the same at every
 * size (`checkWindow` compares them). Null when the window lays no tree.
 */
export function treeFit(root: Element): TreeFit | null {
  const tree = root.querySelector(".skin-tree");
  if (!tree) return null;
  const r = tree.getBoundingClientRect();
  const views = tree.querySelector('[data-skin-role="views"]');
  const v = views ? views.getBoundingClientRect() : null;
  return {
    inWindow: r.left >= -0.5 && r.top >= -0.5 && r.right <= window.innerWidth + 0.5 && r.bottom <= window.innerHeight + 0.5,
    box: {
      left: Math.round(r.left),
      top: Math.round(r.top),
      right: Math.round(r.right),
      bottom: Math.round(r.bottom),
      viewport: [window.innerWidth, window.innerHeight],
    },
    gapWidth: Math.round(window.innerWidth - r.width),
    gapHeight: Math.round(window.innerHeight - r.height),
    viewsGapHeight: v && v.height > 0 ? Math.round(window.innerHeight - v.height) : null,
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
        // A visually hidden announcer (`.sr-only`, 00-rules.css: 1 px,
        // clipped, its -1px margin part of the pattern) draws no spacing;
        // the other checks skip it too (OVERLAYS).
        if (child.matches(".sr-only")) continue;
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
