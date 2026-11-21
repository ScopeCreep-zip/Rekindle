// Plan E7.14 S5 (moved from E7.3's e2e/a11y/axe-skins.spec.ts and E7.12's
// layout specs under each skin): every built-in skin, applied for real
// through Settings → Appearance, then axe at WCAG 2.0–2.2 A and AA (the
// level plan E7.3 targets, wiring guide §13) and the geometry checks
// (E7.12) in the buddy list, Settings → Appearance and a community window
// holding a long channel name and a long unbroken URL, each at its
// minimum, default and large size. A skin recolours and redraws the app,
// so contrast, focus and layout must hold under each one, not only under
// the default. Each window is saved as
// e2e/app/out/skins-a11y/<skin>/<window>-<size>.png.

import { mkdirSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Violation, violations } from "../../axe.ts";
import { checkGeometry } from "../../geometry.ts";
import { applySkin, createCommunity, createIdentity, createTextChannel, newUser, openCommunity, postMessage, selectChannel, type User } from "../../journeys.ts";
import { builtinPacks, type SkinPack } from "../../skin-pack.ts";
import { BUDDY_LIST, COMMUNITY, SETTINGS_APPEARANCE, type WindowCase, sizeWindow, sizes } from "../../windows.ts";

const OUT = path.join(import.meta.dirname, "..", "..", "out", "skins-a11y");
const PACKS = builtinPacks();

/** `prefix` padded to `n` characters: the longest names the app accepts. */
function longName(prefix: string, n = 64): string {
  return (prefix + "W".repeat(n)).slice(0, n);
}

/**
 * One finding as a key: the window, its size and the check, with axe split
 * by rule and the focus check by element ("buddy-list min: axe
 * color-contrast", "buddy-list min: focus button.menu-bar-item \"Rekindle\"
 * no indicator"). The size's pixels are left out: the name says which.
 */
function keysOf(found: Record<string, unknown>): string[] {
  const keys: string[] = [];
  for (const [key, value] of Object.entries(found)) {
    const [where, check] = [key.slice(0, key.indexOf(": ")).replace(/ \d+x\d+$/, ""), key.slice(key.indexOf(": ") + 2)];
    if (check === "axe") for (const v of value as Violation[]) keys.push(`${where}: axe ${v.rule}`);
    else if (check === "focus indicators") for (const f of value as { el: string; why: string }[]) keys.push(`${where}: focus ${f.el} ${f.why}`);
    else keys.push(`${where}: ${check}`);
  }
  return keys.sort();
}

/**
 * Known failures, held strictly (plan E7.14 S5, the lead's ruling
 * 2026-11-20): each skin's findings must be exactly these, so a new one
 * fails and so does a fixed one (drop its key in the fix's merge). Taken
 * from a run, never widened silently; re-baselined against the SK0 round.
 * Taken from round 10 + S5 on macOS (four runs, the same keys apart from
 * UNSETTLED), and from round 12 on Linux (x11):
 * - `window flex contract`: Simple X's buddy list (both).
 * - `ellipsized text …`: SpaceCowboy at min size (both).
 * - `axe color-contrast`: Combat and Alloy (the light bases), buddy list
 *   and community window, on macOS only. The holds are per platform
 *   (the lead's ruling, 2026-11-21), each exact.
 *
 * Why the contrast keys differ (probe 2026-11-21, macOS): axe flags the
 * menubar's "Calls" and "Help" (#8fb9d7 on #e6e6e6, 1.66:1) and the buddy
 * tabs (Communities #8fb9d7 on #ffffff, 2.08:1; the active Friends #3491d8
 * on #ffffff, 3.39:1). Those foregrounds are the elements' own computed
 * `color`, the dark base's text tokens, unchanged under the light skins:
 * the same CSS on either engine. So macOS's result is the true one, an
 * app finding (the menubar and buddy tabs do not take the light skins'
 * text colours). WebKitGTK's axe returns none of them; the likeliest
 * reason is that it cannot resolve a solid background behind them and
 * files them "incomplete", as macOS does for the titlebar and identity
 * bar (`bgGradient`). Linux's incomplete list would confirm it.
 */
const KNOWN_ON: Record<"darwin" | "linux", Record<string, string[]>> = { darwin: {}, linux: {} };
const KNOWN_BOTH: Record<string, string[]> = {
  "spacecowboy": [
    "buddy-list min: ellipsized text showing fewer than 4 letters or digits",
    "community min: ellipsized text showing fewer than 4 letters or digits",
  ],
  "simple-x": [
    "buddy-list default: window flex contract",
    "buddy-list large: window flex contract",
  ],
};
const KNOWN_DARWIN_ONLY: Record<string, string[]> = {
  "combat": [
    "buddy-list default: axe color-contrast",
    "buddy-list large: axe color-contrast",
    "buddy-list min: axe color-contrast",
    "community default: axe color-contrast",
    "community large: axe color-contrast",
    "community min: axe color-contrast",
  ],
  "alloy": [
    "buddy-list default: axe color-contrast",
    "buddy-list large: axe color-contrast",
    "buddy-list min: axe color-contrast",
    "community default: axe color-contrast",
    "community large: axe color-contrast",
    "community min: axe color-contrast",
  ],
};
for (const [file, keys] of Object.entries(KNOWN_BOTH)) {
  KNOWN_ON.darwin[file] = [...keys];
  KNOWN_ON.linux[file] = [...keys];
}
for (const [file, keys] of Object.entries(KNOWN_DARWIN_ONLY)) KNOWN_ON.darwin[file] = [...(KNOWN_ON.darwin[file] ?? []), ...keys].sort();

/** This platform's holds; a platform without a list of its own holds nothing. */
const KNOWN: Record<string, string[]> = process.platform === "darwin" || process.platform === "linux" ? KNOWN_ON[process.platform] : {};

/**
 * Findings that come and go between runs of one build, so neither a pass
 * nor a hold can be strict; passed over by exact text, and logged each
 * run. After Settings opens from the buddy list's menubar (its only
 * route), the Rekindle trigger may keep focus and Kobalte's
 * data-highlighted and take a moved focus back once (present in two runs,
 * absent in two). Owed the owner's by-hand check (mouse and keyboard →
 * Settings → back to the buddy list → Tab); it decides whether these
 * become a fix or go.
 */
const UNSETTLED = new Set(
  ["min", "default", "large"]
    .map((size) => `buddy-list ${size}: focus button.menu-bar-item "Rekindle" no indicator`)
    .concat([`buddy-list min: focus button.xfire-titlebar-btn.xfire-titlebar-btn-minimize "\u2014" not focusable`]),
);

const LONG_URL = `https://example.com/${"averyveryverylongpathsegment".repeat(12)}?q=${"x".repeat(80)}`;

describe("every built-in skin keeps the app accessible and laid out", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("skins"), displayName: longName("Skins_") };
  const community = longName("Community_", 100);
  const channel = longName("channel_", 100);
  let communityHandle = "";

  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, community);
    await openCommunity(browser, community);
    communityHandle = await browser.getWindowHandle();
    await createTextChannel(browser, me, channel);
    await selectChannel(browser, channel);
    await postMessage(browser, me, LONG_URL);
    await postMessage(browser, me, "a second message");
    await openSettings(browser, "Appearance");
  });

  /** Axe and geometry findings in the window `browser` is in, at each of `w`'s sizes, keyed by window and size. */
  async function audit(pack: SkinPack, w: WindowCase, withData: boolean): Promise<Record<string, unknown>> {
    const found: Record<string, unknown> = {};
    const dir = path.join(OUT, pack.file);
    mkdirSync(dir, { recursive: true });
    for (const [sizeName, size] of sizes(w)) {
      await sizeWindow(browser, size);
      await browser.saveScreenshot(path.join(dir, `${w.name}-${sizeName}.png`));
      const where = `${w.name} ${sizeName} ${size.width}x${size.height}`;
      const axe = await violations(browser);
      if (axe.length > 0) found[`${where}: axe`] = axe;
      const geometry = await checkGeometry(browser, ".app-frame", {
        spacing: withData ? [...w.spacing, ...(w.spacingWithData ?? [])] : w.spacing,
        panes: withData ? (w.panes ?? []) : [],
        overlapAllow: w.overlapAllow,
      });
      for (const [check, value] of Object.entries(geometry)) found[`${where}: ${check}`] = value;
    }
    return found;
  }

  for (const pack of PACKS) {
    const held = (KNOWN[pack.file] ?? []).length;
    it(`${pack.name}: axe and geometry hold in the buddy list, Appearance and a community window${held ? ` (${held} known failure${held === 1 ? "" : "s"})` : ""}`, async () => {
      await applySkin(browser, me, pack.name);
      const found: Record<string, unknown> = {};
      Object.assign(found, await audit(pack, SETTINGS_APPEARANCE, true));
      await switchTo(browser, BUDDY);
      Object.assign(found, await audit(pack, BUDDY_LIST, false));
      await browser.switchToWindow(communityHandle);
      Object.assign(found, await audit(pack, COMMUNITY, true));
      const all = keysOf(found);
      // The run's findings as keys, to re-baseline from (no silent widening: a changed hold is a reviewed edit).
      console.log(`[keys] ${pack.file} ${JSON.stringify(all)}`);
      const unsettled = all.filter((k) => UNSETTLED.has(k));
      if (unsettled.length > 0) console.log(`[unsettled] ${pack.file} ${JSON.stringify(unsettled)}`);
      expect(all.filter((k) => !UNSETTLED.has(k))).toEqual(KNOWN[pack.file] ?? []);
    });
  }
});
