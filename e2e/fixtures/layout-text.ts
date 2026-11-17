import { expect, type Locator, type Page } from "@playwright/test";
import { OVERLAYS, install } from "./layout";

// More geometry assertions for the layout suite (plan E7.12): what an
// ellipsis leaves readable, where the titlebar title ends, and whether the
// composer's controls share one centre line.

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
 * Every ellipsized element under `root` that does cut its text still shows
 * at least `MIN_ELLIPSIS_CHARS` letters or digits (fewer only when its whole
 * text has fewer).
 */
export async function assertEllipsisLegible(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl, [skip, min]) => {
    const g = (window as any).__geo;
    const skipSel = (skip as string[]).join(", ");
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
      if (shown < Math.min(min as number, total)) {
        out.push(`${g.describe(el)} shows ${shown} of ${total} letters/digits in ${el.clientWidth}px`);
      }
    }
    return out.slice(0, 12);
  }, [OVERLAYS, MIN_ELLIPSIS_CHARS] as const);
  expect.soft(bad, `ellipsized text showing fewer than ${MIN_ELLIPSIS_CHARS} letters or digits`).toEqual([]);
}

/**
 * The share of the titlebar's content width the title may take (the rule
 * `.xfire-titlebar-text { max-width: 50% }`): a long name ellipsizes in the
 * left half, like the in-pane headers, instead of running into the window
 * controls.
 */
const TITLE_SHARE = 0.5;

/**
 * Each window's titlebar title ends within `TITLE_SHARE` of the bar and,
 * when its text is longer than that box, is cut there with an ellipsis.
 */
export async function assertTitlebarTitleBounded(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl, share) => {
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
  }, TITLE_SHARE);
  expect.soft(bad, "titlebar title not bounded with an ellipsis").toEqual([]);
}

/** How far apart (CSS px) the composer's controls' vertical centres may sit. */
const CENTRE_TOLERANCE = 2;

/** The composer's in-flow controls (buttons and the text field) share one centre line. */
export async function assertComposerAligned(page: Page, root: Locator): Promise<void> {
  await install(page);
  const bad = await root.evaluate((rootEl, tol) => {
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
  }, CENTRE_TOLERANCE);
  expect.soft(bad, "composer controls off one centre line").toEqual([]);
}
