// Plan E7.14 S4: E7.12's geometry checks in the real window. The checks
// themselves are page-side functions (geometry-page.ts, geometry-page-text.ts); this runs each one
// inside the window a session is on, through WebDriver's execute, and
// returns every finding at once, so a failure names all of a window's
// faults (Playwright's `expect.soft` did the same).

import { isDeepStrictEqual } from "node:util";
import type { Browser } from "webdriverio";
import {
  CENTRE_TOLERANCE,
  MIN_ELLIPSIS_CHARS,
  MIN_FIELD_WIDTH,
  MIN_TARGET,
  TITLE_SHARE,
  badgesOutOfRow,
  composerMisaligned,
  ellipsisIllegible,
  fieldsSqueezed,
  panesScrollingSideways,
  placeholderText,
  textClipped,
  timestampsOffLine,
  titlebarTargetsSmall,
  titlebarTitleUnbounded,
} from "./geometry-page-text.ts";
import {
  FLEX_CONTRACT,
  OVERLAYS,
  flexContract,
  focusIndicators,
  geometryLib,
  horizontalOverflow,
  overlaps,
  spacingOffScale,
  textOverflows,
  type TreeFit,
  treeFit,
} from "./geometry-page.ts";

/**
 * The suite's TypeScript is loaded by tsx, whose esbuild `keepNames` wraps
 * each named inner function as `__name(fn, "name")` and uses the result
 * (checked: `geometryLib.toString()` contains `__name` under tsx 4.23.15).
 * The page has no such helper, so the source sent in defines one that
 * returns the function unchanged; a name is never read in the page.
 */
const PRELUDE = "var __name = function (fn) { return fn; };";

/**
 * Run `check(root, arg)` in the session's current window, `root` being the
 * first element matching `rootSelector`, after installing the helpers.
 * Sent as a script body, so the page's CSP never meets an eval.
 */
async function inPage<A, R>(b: Browser, rootSelector: string, check: (root: Element, arg: A) => R, arg: A): Promise<R> {
  const script = `${PRELUDE}
(${geometryLib.toString()})();
var root = document.querySelector(${JSON.stringify(rootSelector)});
if (!root) throw new Error(${JSON.stringify(`geometry root ${rootSelector} not found`)});
return (${check.toString()})(root, ${JSON.stringify(arg)});`;
  return (await b.execute(script)) as R;
}

/** What a window case declares beyond its root (E7.12's `WindowCase`). */
export interface GeometryOptions {
  /** Containers whose declared gaps must sit on the spacing grid. */
  spacing: string[];
  /** Content panes that must never scroll sideways. */
  panes: string[];
  /** More overlays this case shows on purpose (a menu held open). */
  allow?: string[];
  /**
   * Elements that overlap their neighbour by design, exempt from the
   * overlap check only (every other check still measures them): the bell's
   * corner count badge, which sits on the glyph (lead decision O1,
   * 2026-11-19).
   */
  overlapAllow?: string[];
  /**
   * The window is on screen, where the user's real pointer may rest over it
   * and set `:hover` (a window the driver cannot place off-screen: native
   * Wayland, where a client cannot position itself). Hover reveals and
   * restyles elements (a message's action bar over its timestamp, S4
   * 2026-11-19), so the checks of what is rendered are left out: overlaps,
   * text that neither fits nor ellipsizes, sideways clipping and illegible
   * ellipses. The structure checks still run.
   */
  onScreen?: boolean;
}

/** How the window's skin tree sits in it (`treeFit`), or null when it lays none. */
export async function treeFitIn(b: Browser, rootSelector: string): Promise<TreeFit | null> {
  return inPage(b, rootSelector, treeFit, null);
}

/** Every check's findings, keyed by check; `{}` when the window passes. */
export type Findings = Record<string, unknown>;

/**
 * All of E7.12's checks on the window `b` is in, in its order, with the
 * root `rootSelector` (each window's `.app-frame`). Focus runs late: it
 * moves focus and scrolls each control into view.
 */
export async function checkGeometry(b: Browser, rootSelector: string, opts: GeometryOptions): Promise<Findings> {
  const skip = [...OVERLAYS, ...(opts.allow ?? [])];
  const found: Findings = {};
  const record = (name: string, value: unknown, ok: unknown): void => {
    if (!isDeepStrictEqual(value, ok)) found[name] = value;
  };
  // A skin's tree places every node itself, so it keeps the tree's fit in
  // place of the flex contract (its growth is compared across sizes by
  // the caller, `treeFitIn`).
  const fit = await treeFitIn(b, rootSelector);
  // The box rides along in a finding only (the pass value stays `true`).
  if (fit) record("tree inside the window", fit.inWindow ? true : fit.box, true);
  else record("window flex contract", await inPage(b, rootSelector, flexContract, null), FLEX_CONTRACT);
  record("horizontal overflow (px)", await inPage(b, rootSelector, horizontalOverflow, null), { document: 0, root: 0 });
  if (!opts.onScreen) record("overlapping elements", await inPage(b, rootSelector, overlaps, [...skip, ...(opts.overlapAllow ?? [])]), []);
  if (!opts.onScreen) record("text that neither fits nor ellipsizes", await inPage(b, rootSelector, textOverflows, skip), []);
  if (!opts.onScreen) record("text clipped sideways without an ellipsis", await inPage(b, rootSelector, textClipped, skip), []);
  record("content panes scrolling sideways", await inPage(b, rootSelector, panesScrollingSideways, opts.panes), []);
  record("badges outside their row", await inPage(b, rootSelector, badgesOutOfRow, null), []);
  record("missing values shown as text", await inPage(b, rootSelector, placeholderText, null), []);
  record("timestamps off their line", await inPage(b, rootSelector, timestampsOffLine, null), []);
  record("text fields squeezed", await inPage(b, rootSelector, fieldsSqueezed, MIN_FIELD_WIDTH), []);
  record("spacing off the grid", await inPage(b, rootSelector, spacingOffScale, opts.spacing), []);
  if (!opts.onScreen) {
    record(
      `ellipsized text showing fewer than ${MIN_ELLIPSIS_CHARS} letters or digits`,
      await inPage(b, rootSelector, ellipsisIllegible, [OVERLAYS, MIN_ELLIPSIS_CHARS] as [string[], number]),
      [],
    );
  }
  record("titlebar title not bounded with an ellipsis", await inPage(b, rootSelector, titlebarTitleUnbounded, TITLE_SHARE), []);
  record("composer controls off one centre line", await inPage(b, rootSelector, composerMisaligned, CENTRE_TOLERANCE), []);
  record("focus indicators", await inPage(b, rootSelector, focusIndicators, skip), []);
  record(`titlebar button under ${MIN_TARGET}x${MIN_TARGET}`, await inPage(b, rootSelector, titlebarTargetsSmall, MIN_TARGET), []);
  return found;
}
