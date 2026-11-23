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
import { MOD_KEY, byText, switchTo } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, expectOnlySelectedTab, openSettings } from "../../../walk/nav.ts";
import { type Violation, violations } from "../../axe.ts";
import { checkGeometry, targetsMetIn, treeFitIn } from "../../geometry.ts";
import type { TreeFit } from "../../geometry-page.ts";
import { applySkin, createCommunity, createIdentity, createTextChannel, newUser, openCommunity, postMessage, selectChannel, skinShown, type User } from "../../journeys.ts";
import { builtinPacks, type SkinPack } from "../../skin-pack.ts";
import { BUDDY_LIST, COMMUNITY, SETTINGS_APPEARANCE, type WindowCase, sizeWindow, sizesIn, spacingOf, treeGrowth } from "../../windows.ts";

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
    else if (check === "target-size short" || check === "target-size") for (const id of value as string[]) keys.push(`${where}: ${check} ${id}`);
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
/**
 * Faithful skin; conforming alternate = builtin:symbiosis-access (plan SK14,
 * WCAG 2.2 5.2.1 conforming alternate version). The Xfire skins keep their
 * art's small controls: each served-short control's 24 px target would cross
 * a neighbour's, and none meets 2.5.8's spacing exception at its press box
 * (targetsMet). The accessible variant, one keyboard step from every skin
 * (Mod+,), holds none and must fully pass. Pinned from round 26's run
 * (3b260dba2, 2026-11-23); a changed list is a reviewed edit.
 */
const KNOWN_FAITHFUL_SHORT: Record<string, string[]> = {
  "symbiosis": [
    "buddy-list default: target-size short ButtonClose",
    "buddy-list default: target-size short ButtonMaximize",
    "buddy-list default: target-size short StatusChanger",
    "buddy-list large: target-size short ButtonClose",
    "buddy-list large: target-size short ButtonMaximize",
    "buddy-list large: target-size short StatusChanger",
    "buddy-list min: target-size short ButtonClose",
    "buddy-list min: target-size short ButtonMaximize",
    "buddy-list min: target-size short StatusChanger",
  ],
  "xfire-classic": [
    "buddy-list default: target-size short DownArrow",
    "buddy-list large: target-size short DownArrow",
    "buddy-list min: target-size short DownArrow",
  ],
  "combat": [
    "buddy-list default: target-size short DownArrow",
    "buddy-list large: target-size short DownArrow",
    "buddy-list min: target-size short DownArrow",
  ],
  "onyx": [
    "buddy-list default: target-size short Close",
    "buddy-list default: target-size short DownArrow",
    "buddy-list default: target-size short ExpandButton",
    "buddy-list default: target-size short FileMenu",
    "buddy-list default: target-size short HelpMenu",
    "buddy-list default: target-size short Maximize",
    "buddy-list default: target-size short Minimize",
    "buddy-list default: target-size short TabNormalButton",
    "buddy-list default: target-size short ToolsMenu",
    "buddy-list large: target-size short Close",
    "buddy-list large: target-size short DownArrow",
    "buddy-list large: target-size short ExpandButton",
    "buddy-list large: target-size short FileMenu",
    "buddy-list large: target-size short HelpMenu",
    "buddy-list large: target-size short Maximize",
    "buddy-list large: target-size short Minimize",
    "buddy-list large: target-size short TabNormalButton",
    "buddy-list large: target-size short ToolsMenu",
    "buddy-list min: target-size short Close",
    "buddy-list min: target-size short DownArrow",
    "buddy-list min: target-size short ExpandButton",
    "buddy-list min: target-size short FileMenu",
    "buddy-list min: target-size short HelpMenu",
    "buddy-list min: target-size short Maximize",
    "buddy-list min: target-size short Minimize",
    "buddy-list min: target-size short TabNormalButton",
    "buddy-list min: target-size short ToolsMenu",
  ],
  "spacecowboy": [
    "buddy-list default: target-size short Close",
    "buddy-list default: target-size short DownArrow",
    "buddy-list default: target-size short FileMenu",
    "buddy-list default: target-size short HelpMenu",
    "buddy-list default: target-size short Maximize",
    "buddy-list default: target-size short Minimize",
    "buddy-list default: target-size short ToolsMenu",
    "buddy-list large: target-size short Close",
    "buddy-list large: target-size short DownArrow",
    "buddy-list large: target-size short FileMenu",
    "buddy-list large: target-size short HelpMenu",
    "buddy-list large: target-size short Maximize",
    "buddy-list large: target-size short Minimize",
    "buddy-list large: target-size short ToolsMenu",
    "buddy-list min: target-size short Close",
    "buddy-list min: target-size short DownArrow",
    "buddy-list min: target-size short FileMenu",
    "buddy-list min: target-size short HelpMenu",
    "buddy-list min: target-size short Maximize",
    "buddy-list min: target-size short Minimize",
    "buddy-list min: target-size short ToolsMenu",
  ],
  "stealthx": [
    "buddy-list default: target-size short StatusBtn",
    "buddy-list default: target-size short WndClose",
    "buddy-list default: target-size short WndMax",
    "buddy-list default: target-size short WndMin",
    "buddy-list large: target-size short StatusBtn",
    "buddy-list large: target-size short WndClose",
    "buddy-list large: target-size short WndMax",
    "buddy-list large: target-size short WndMin",
    "buddy-list min: target-size short StatusBtn",
    "buddy-list min: target-size short WndClose",
    "buddy-list min: target-size short WndMax",
    "buddy-list min: target-size short WndMin",
  ],
  "total-chaos": [
    "buddy-list default: target-size short Close",
    "buddy-list default: target-size short Maximize",
    "buddy-list default: target-size short Minimize",
    "buddy-list large: target-size short Close",
    "buddy-list large: target-size short Maximize",
    "buddy-list large: target-size short Minimize",
    "buddy-list min: target-size short Close",
    "buddy-list min: target-size short Maximize",
    "buddy-list min: target-size short Minimize",
  ],
  "xslimfire": [
    "buddy-list default: target-size short CloseBox",
    "buddy-list default: target-size short MinimizeBox",
    "buddy-list default: target-size short StatusEdit",
    "buddy-list large: target-size short CloseBox",
    "buddy-list large: target-size short MinimizeBox",
    "buddy-list large: target-size short StatusEdit",
    "buddy-list min: target-size short CloseBox",
    "buddy-list min: target-size short MinimizeBox",
    "buddy-list min: target-size short StatusEdit",
  ],
  "alloy": [
    "buddy-list default: target-size short CloseBox",
    "buddy-list default: target-size short MaximizeBox",
    "buddy-list default: target-size short MinimizeBox",
    "buddy-list large: target-size short CloseBox",
    "buddy-list large: target-size short MaximizeBox",
    "buddy-list large: target-size short MinimizeBox",
    "buddy-list min: target-size short CloseBox",
    "buddy-list min: target-size short MaximizeBox",
    "buddy-list min: target-size short MinimizeBox",
  ],
  "simple-x": [
    "buddy-list default: target-size short CloseBox",
    "buddy-list default: target-size short HelpMenu",
    "buddy-list default: target-size short MaximizeBox",
    "buddy-list default: target-size short MinimizeBox",
    "buddy-list large: target-size short CloseBox",
    "buddy-list large: target-size short HelpMenu",
    "buddy-list large: target-size short MaximizeBox",
    "buddy-list large: target-size short MinimizeBox",
    "buddy-list min: target-size short CloseBox",
    "buddy-list min: target-size short HelpMenu",
    "buddy-list min: target-size short MaximizeBox",
    "buddy-list min: target-size short MinimizeBox",
  ],
};
for (const [file, keys] of Object.entries(KNOWN_BOTH)) {
  KNOWN_ON.darwin[file] = [...keys];
  KNOWN_ON.linux[file] = [...keys];
}
for (const [file, keys] of Object.entries(KNOWN_FAITHFUL_SHORT)) {
  for (const os of ["darwin", "linux"] as const) KNOWN_ON[os][file] = [...(KNOWN_ON[os][file] ?? []), ...keys].sort();
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
    .concat([`buddy-list min: focus button.xfire-titlebar-btn.xfire-titlebar-btn-minimize "\u2014" not focusable`])
    // The same take-back in a skin's tree: Symbiosis's first control after
    // the menu bar (its logo is a label since plan BR), round 26.
    .concat([`buddy-list min: focus div.skin-tree-button[ButtonMinimize] not focusable`]),
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

  /**
   * Axe's undersized targets split by the rule the Host serves each one
   * (plan SK7 `NodeTarget.rule`): `short` nodes by their skin node id, the
   * rest by id or, outside the tree, their selector. A tree control axe
   * flags whose press box (plan SK12: a cut area takes its press on a
   * `::before`, its art box none) meets WCAG 2.5.8 (`met`) is no finding.
   */
  async function targetNodes(selectors: string[], met: string[]): Promise<{ short: string[]; other: string[] }> {
    return (await browser.execute(async (sels: string[], met: string[]) => {
      type Node = { id: string; target?: { rule: string }; children?: Node[] };
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<{ value: { shells?: { root: Node[] }[] } }> } };
      const rules = new Map<string, string>();
      const walk = (n: Node): void => {
        if (n.target) rules.set(n.id, n.target.rule);
        for (const c of n.children ?? []) walk(c);
      };
      for (const shell of (await w.__TAURI_INTERNALS__.invoke("get_skin", { id: null })).value.shells ?? []) shell.root.forEach(walk);
      const short: string[] = [];
      const other: string[] = [];
      for (const sel of sels) {
        const id = document.querySelector(sel)?.closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode;
        if (id && rules.get(id) === "short") short.push(id);
        else if (!met.includes(sel)) other.push(id ?? sel);
      }
      return { short: short.sort(), other: other.sort() };
    }, selectors, met)) as { short: string[]; other: string[] };
  }

  /**
   * The buddy list's labelled controls whose name or tooltip is not the
   * label's words (plan SK14, WCAG 2.5.3): a pressable node and a menu title
   * the Host labels are named by those words, and a pressable node's title
   * repeats them (the full words where its box cuts them short).
   */
  async function labelNames(): Promise<string[]> {
    return (await browser.execute(() =>
      Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")).flatMap((el) => {
        if (!el.matches(".skin-tree-button, .skin-menu")) return [];
        const label = el.querySelector(":scope > .skin-node-label");
        const words = (label?.querySelector(".skin-node-label-text, .sr-only")?.textContent ?? "").trim();
        if (!words) return [];
        const id = el.dataset.skinNode ?? "?";
        const out: string[] = [];
        const name = el.getAttribute("aria-label") ?? "";
        if (name !== words) out.push(`${id} named "${name}", shows "${words}"`);
        if (el.matches(".skin-tree-button") && el.getAttribute("title") !== words) out.push(`${id} titled "${el.getAttribute("title") ?? ""}"`);
        return out;
      }),
    )) as string[];
  }

  /**
   * Whether Mod+, in the buddy list opens Settings at Appearance (plan SK14):
   * Settings is moved to About first, so only the chord can bring it back.
   */
  async function appearanceChordOpens(): Promise<boolean> {
    await switchTo(browser, SETTINGS);
    await (await byText(browser, ".form-tab", "About")).click();
    await switchTo(browser, BUDDY);
    await browser.keys([MOD_KEY, ","]);
    await switchTo(browser, SETTINGS);
    try {
      await expectOnlySelectedTab(browser, "Appearance");
      return true;
    } catch {
      return false;
    }
  }

  /** Axe and geometry findings in the window `browser` is in, at each of `w`'s sizes, keyed by window and size. */
  async function audit(pack: SkinPack, w: WindowCase, withData: boolean): Promise<Record<string, unknown>> {
    const found: Record<string, unknown> = {};
    const dir = path.join(OUT, pack.file);
    mkdirSync(dir, { recursive: true });
    const fits: [string, TreeFit][] = [];
    for (const [sizeName, size] of await sizesIn(browser, w)) {
      await sizeWindow(browser, size);
      await browser.saveScreenshot(path.join(dir, `${w.name}-${sizeName}.png`));
      const where = `${w.name} ${sizeName} ${size.width}x${size.height}`;
      const all = await violations(browser);
      // Target size per node (plan SK7): a node the Host serves `short` (its
      // 24 px target would cross a neighbour's, pinned in targets-short.txt)
      // is named as such; any other undersized node is a regression.
      const axe = all.filter((v) => v.rule !== "target-size");
      if (axe.length > 0) found[`${where}: axe`] = axe;
      const small = all.find((v) => v.rule === "target-size");
      if (small) {
        const { short, other } = await targetNodes(small.nodes, await targetsMetIn(browser, ".app-frame", small.nodes));
        if (short.length > 0) found[`${where}: target-size short`] = short;
        if (other.length > 0) found[`${where}: target-size`] = other;
      }
      const fit = await treeFitIn(browser, ".app-frame");
      if (fit) fits.push([where, fit]);
      const geometry = await checkGeometry(browser, ".app-frame", {
        spacing: spacingOf(w, fit !== null, withData),
        panes: withData ? (w.panes ?? []) : [],
        overlapAllow: w.overlapAllow,
      });
      for (const [check, value] of Object.entries(geometry)) found[`${where}: ${check}`] = value;
    }
    const growth = treeGrowth(fits);
    if (growth.length > 0) found[`${w.name}: tree does not grow with the window`] = growth;
    return found;
  }

  for (const pack of PACKS) {
    const held = (KNOWN[pack.file] ?? []).length;
    it(`${pack.name}: axe and geometry hold in the buddy list, Appearance and a community window${held ? ` (${held} known failure${held === 1 ? "" : "s"})` : ""}`, async () => {
      const href = await applySkin(browser, me, pack.name);
      const found: Record<string, unknown> = {};
      Object.assign(found, await audit(pack, SETTINGS_APPEARANCE, true));
      // Each window is audited once it shows this skin (its own link, SK4).
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      Object.assign(found, await audit(pack, BUDDY_LIST, false));
      // Plan SK14: label words name their control, and the accessible skin
      // is one keyboard step away.
      const misnamed = await labelNames();
      if (misnamed.length > 0) found["buddy-list: label words not the control's name"] = misnamed;
      if (!(await appearanceChordOpens())) found["buddy-list: Mod+, does not open Appearance"] = true;
      await browser.switchToWindow(communityHandle);
      await skinShown(browser, href);
      Object.assign(found, await audit(pack, COMMUNITY, true));
      const all = keysOf(found);
      // The run's findings as keys, to re-baseline from (no silent widening: a changed hold is a reviewed edit).
      console.log(`[keys] ${pack.file} ${JSON.stringify(all)}`);
      // The values behind each finding not held, so a failure names its elements.
      const held = new Set(KNOWN[pack.file] ?? []);
      const detail = Object.entries(found).filter(([k, v]) => keysOf({ [k]: v }).some((key) => !held.has(key)));
      if (detail.length > 0) console.log(`[detail] ${pack.file} ${JSON.stringify(Object.fromEntries(detail)).slice(0, 20_000)}`);
      const unsettled = all.filter((k) => UNSETTLED.has(k));
      if (unsettled.length > 0) console.log(`[unsettled] ${pack.file} ${JSON.stringify(unsettled)}`);
      expect(all.filter((k) => !UNSETTLED.has(k))).toEqual(KNOWN[pack.file] ?? []);
    });
  }
});
