import { expect, type Locator, type Page } from "@playwright/test";

// Geometry assertions for the layout suite (plan E7.12). They read the
// rendered boxes in the page, so a failure names the elements and their
// boxes, not a pixel diff. Each returns nothing and fails through `expect`.

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
function geometryLib(): void {
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

async function install(page: Page): Promise<void> {
  await page.evaluate(`(${geometryLib.toString()})()`);
}

/**
 * No two visible interactive or text elements under `root` intersect by
 * more than a pixel each way, unless one contains the other or one sits in
 * a declared overlay (`OVERLAYS` plus `allow`).
 */
export async function assertNoOverlap(page: Page, root: Locator, opts: { allow?: string[] } = {}): Promise<void> {
  await install(page);
  const skip = [...OVERLAYS, ...(opts.allow ?? [])];
  const clashes = await root.evaluate((rootEl, skip) => {
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
  }, skip);
  expect.soft(clashes, "overlapping elements").toEqual([]);
}

/** Neither the document nor `root` scrolls sideways. */
export async function assertNoHorizontalOverflow(page: Page, root: Locator): Promise<void> {
  const over = await root.evaluate((el) => {
    const doc = document.scrollingElement!;
    return {
      document: doc.scrollWidth - doc.clientWidth,
      root: el.scrollWidth - el.clientWidth,
    };
  });
  expect.soft(over, "horizontal overflow (px)").toEqual({ document: 0, root: 0 });
}

/**
 * Every visible block-level text element fits its box or shows the declared
 * ellipsis. Form fields and declared scrollers (`overflow-x: auto|scroll`)
 * scroll by design.
 */
export async function assertTextFits(page: Page, root: Locator, opts: { allow?: string[] } = {}): Promise<void> {
  await install(page);
  const skip = [...OVERLAYS, ...(opts.allow ?? [])];
  const bad = await root.evaluate((rootEl, skip) => {
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
  }, skip);
  expect.soft(bad, "text that neither fits nor ellipsizes").toEqual([]);
}

/**
 * The window root is a full-height flex column with exactly one growing
 * child, and its last in-flow child ends at the window's bottom edge.
 */
export async function assertFlexContract(page: Page, root: Locator): Promise<void> {
  const contract = await root.evaluate((el) => {
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
  });
  expect.soft(contract, "window flex contract").toEqual({
    display: "flex",
    direction: "column",
    fillsWindow: true,
    growing: 1,
    footerAtBottom: true,
  });
}

/**
 * Every focusable element under `root` matches `:focus-visible` when
 * focused from the keyboard and looks different focused (outline, shadow,
 * border, background or text colour), and a ring it draws (outline or
 * box-shadow) is not cut by a clipping ancestor.
 */
export async function assertFocusVisibleNotClipped(page: Page, root: Locator, opts: { allow?: string[] } = {}): Promise<void> {
  await install(page);
  // Keyboard modality, so programmatic focus matches :focus-visible.
  await page.keyboard.press("Shift");
  const skip = [...OVERLAYS, ...(opts.allow ?? [])];
  const bad = await root.evaluate((rootEl, skip) => {
    const g = (window as any).__geo;
    const skipSel = skip.join(", ");
    const out: { el: string; why: string }[] = [];
    const els = Array.from(rootEl.querySelectorAll(g.INTERACTIVE)) as HTMLElement[];
    for (const el of els) {
      if (skipSel && el.closest(skipSel)) continue;
      if (!g.shown(el) || (el as HTMLButtonElement).disabled) continue;
      el.scrollIntoView({ block: "nearest", inline: "nearest" });
      const look = () => {
        const s = getComputedStyle(el);
        return [s.outlineStyle, s.outlineWidth, s.outlineColor, s.boxShadow, s.backgroundColor,
          s.borderTopColor, s.borderRightColor, s.borderBottomColor, s.borderLeftColor, s.color].join("|");
      };
      const before = look();
      el.focus();
      if (document.activeElement !== el) { out.push({ el: g.describe(el), why: "not focusable" }); continue; }
      if (!el.matches(":focus-visible")) { out.push({ el: g.describe(el), why: "no :focus-visible" }); continue; }
      const s = getComputedStyle(el);
      if (look() === before) { out.push({ el: g.describe(el), why: "no indicator" }); continue; }
      const ow = s.outlineStyle === "none" ? 0 : parseFloat(s.outlineWidth) || 0;
      if (ow === 0 && s.boxShadow === "none") continue;
      const grow = Math.max(0, ow + (parseFloat(s.outlineOffset) || 0));
      const r = el.getBoundingClientRect();
      const ring = { x: r.left - grow, y: r.top - grow, w: r.width + 2 * grow, h: r.height + 2 * grow };
      const seen = g.visibleRect(el.parentElement ?? el, ring);
      if (!seen || seen.w + 0.5 < ring.w || seen.h + 0.5 < ring.h) {
        out.push({ el: g.describe(el), why: `ring clipped to ${seen ? `${seen.w.toFixed(1)}x${seen.h.toFixed(1)}` : "nothing"} of ${ring.w.toFixed(1)}x${ring.h.toFixed(1)}` });
      }
    }
    (document.activeElement as HTMLElement | null)?.blur();
    return out.slice(0, 12);
  }, skip);
  expect.soft(bad, "focus indicators").toEqual([]);
}

/**
 * The gaps the stylesheet declares on `containers` (computed `row-gap`,
 * `column-gap`, and each child's non-`auto` margins) sit on the spacing
 * grid: whole multiples of `--layout-grid` (`src/styles/global.css`). The
 * step is read from the page, never written here.
 */
export async function assertSpacingOnScale(page: Page, root: Locator, containers: string[]): Promise<void> {
  const off = await root.evaluate((rootEl, containers) => {
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
  }, containers);
  expect.soft(off, "spacing off the grid").toEqual([]);
}

/**
 * No visible text under `root` is cut sideways by an ancestor (or itself)
 * that clips (`overflow` hidden, clip, auto or scroll), unless an element
 * between the text and that clipper ellipsizes (`text-overflow: ellipsis`,
 * `overflow: hidden`, `white-space: nowrap`) and is itself wholly inside
 * the clipper, so the reader sees the ellipsis. Boxes are compared
 * unclipped: the clipping is what this looks for.
 */
export async function assertTextNotClipped(page: Page, root: Locator, opts: { allow?: string[] } = {}): Promise<void> {
  await install(page);
  const skip = [...OVERLAYS, ...(opts.allow ?? [])];
  const bad = await root.evaluate((rootEl, skip) => {
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
  }, skip);
  expect.soft(bad, "text clipped sideways without an ellipsis").toEqual([]);
}

/** The content panes under `root` that render (`panes`) never scroll sideways. */
export async function assertPanesNoHorizontalScroll(page: Page, root: Locator, panes: string[]): Promise<void> {
  const over = await root.evaluate((rootEl, panes) => {
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
  }, panes);
  expect.soft(over, "content panes scrolling sideways").toEqual([]);
}

/**
 * Badges and counts sit wholly inside their row: never squeezed below
 * their content, not cut sideways by a clipping ancestor or the window,
 * nor spilling out of their row.
 */
export async function assertBadgesInRow(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl) => {
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
  });
  expect.soft(bad, "badges outside their row").toEqual([]);
}

/** No visible text or accessible label under `root` shows a missing value. */
export async function assertNoPlaceholderText(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl) => {
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
  });
  expect.soft(bad, "missing values shown as text").toEqual([]);
}

/**
 * A timestamp is one line and shares that line with the element before it
 * (a message header's sender name), never wrapping under it.
 */
export async function assertTimestampsInline(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl) => {
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
  });
  expect.soft(bad, "timestamps off their line").toEqual([]);
}

/** The narrowest a text field may render and still be typed into. */
const MIN_FIELD_WIDTH = 80;

/** Every visible text field under `root` is at least `MIN_FIELD_WIDTH` wide, never squeezed by its row. */
export async function assertFieldsUsable(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl, min) => {
    const g = (window as any).__geo;
    const out: string[] = [];
    const sel = "input:not([type]), input[type=text], input[type=search], input[type=password], input[type=email], input[type=url], textarea, select";
    for (const el of Array.from(rootEl.querySelectorAll(sel))) {
      if (!g.shown(el)) continue;
      const w = el.getBoundingClientRect().width;
      if (w < min) out.push(`${g.describe(el)}${el.id ? `#${el.id}` : ""} is ${w.toFixed(1)}px wide`);
    }
    return out.slice(0, 12);
  }, MIN_FIELD_WIDTH);
  expect.soft(bad, "text fields squeezed").toEqual([]);
}
