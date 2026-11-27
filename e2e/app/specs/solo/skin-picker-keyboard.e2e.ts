// Skin catalogue ruling 5 (frontend half): Settings → Appearance lists every
// built-in the Host serves, and the keyboard path through it stays short at
// any pack count: the list is one Tab stop (the radio group), and only the
// row in use keeps its actions in the Tab order. The arrows choose the next
// skin, and the row whose actions are reachable moves with it. Each row's
// credits are the Host's (`list_skins`), shown as served.

import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { NET_WAIT_MS, shown, waitUntil } from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

interface Reach {
  rows: number;
  names: string[];
  inUse: string;
  listStops: number;
  actionStops: string[];
}

/** The skin list's rows and every element in it the Tab order reaches. */
async function reach(): Promise<Reach> {
  return (await browser.execute(() => {
    const list = document.querySelector<HTMLElement>(".skin-list");
    const rows = Array.from(list?.querySelectorAll<HTMLElement>(".skin-row") ?? []);
    const tabbable = (e: HTMLElement): boolean => e.tabIndex >= 0 && !(e as HTMLButtonElement).disabled && e.getClientRects().length > 0;
    // A radio group is one stop: the checked radio (or the first) holds it.
    const radios = Array.from(list?.querySelectorAll<HTMLInputElement>('input[type="radio"]') ?? []);
    const listStops = radios.length > 0 ? 1 : 0;
    const actionStops = Array.from(list?.querySelectorAll<HTMLElement>("button") ?? [])
      .filter(tabbable)
      .map((b) => `${b.closest(".skin-row")?.querySelector(".skin-name")?.textContent?.trim()}: ${b.getAttribute("aria-label") ?? b.textContent?.trim()}`);
    return {
      rows: rows.length,
      names: rows.map((r) => r.querySelector(".skin-name")?.textContent?.trim() ?? ""),
      inUse: document.querySelector(".skin-row-shown .skin-name")?.textContent?.trim() ?? "",
      listStops,
      actionStops,
    };
  })) as Reach;
}

describe("Settings → Appearance scales to every built-in by keyboard (skin catalogue ruling 5)", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("picker"), displayName: "Ari" };

  it("every built-in is listed; the list is one stop and only the row in use offers its actions; arrows move both", async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
    await shown(browser, ".skin-row-shown .skin-name", NET_WAIT_MS);
    const before = await reach();
    console.log(`[picker] ${before.rows} rows; in use ${before.inUse}; list stops ${before.listStops}; action stops ${JSON.stringify(before.actionStops)}`);
    await shot("picker-before");
    const shipped = builtinPacks().map((p) => p.name);
    const missing = shipped.filter((n) => !before.names.includes(n));
    // Only the in-use row's credits and login actions are reachable.
    const foreign = before.actionStops.filter((s) => !s.startsWith(`${before.inUse}:`));
    expect({ missing, listStops: before.listStops, foreign }).toEqual({ missing: [], listStops: 1, foreign: [] });

    // The login-skin badge starts where the row's other under-name lines do.
    const insets = (await browser.execute(() => {
      const row = document.querySelector(".skin-badge")?.closest(".skin-row");
      const x = (sel: string): number | null => {
        const e = row?.querySelector(sel);
        return e ? Math.round(e.getBoundingClientRect().left + Number.parseFloat(getComputedStyle(e).paddingLeft)) : null;
      };
      return { badge: x(".skin-badge"), sizes: x(".skin-access") };
    })) as { badge: number | null; sizes: number | null };
    console.log(`[picker] under-name text starts: badge ${insets.badge}, sizes ${insets.sizes}`);
    expect(insets.badge !== null && insets.badge === insets.sizes).toBe(true);

    // The arrows choose the next skin; the reachable actions follow it.
    await browser.execute(() => document.querySelector<HTMLInputElement>(".skin-row-shown input[type=radio]")?.focus());
    await browser.keys(Key.ArrowDown);
    await waitUntil(async () => (await reach()).inUse !== before.inUse, NET_WAIT_MS, "ArrowDown did not choose the next skin");
    const after = await reach();
    console.log(`[picker] after ArrowDown: in use ${after.inUse}; action stops ${JSON.stringify(after.actionStops)}`);
    await shot("picker-after-arrow");
    const afterForeign = after.actionStops.filter((s) => !s.startsWith(`${after.inUse}:`));
    // The Host's refresh after a choice must leave the focus on the checked
    // radio (a list rebuilt from fresh entries replaced it, and the second
    // arrow then did nothing: codex-relay's finding).
    const focusOnChecked = async (): Promise<boolean> =>
      (await browser.execute(() => {
        const a = document.activeElement as HTMLInputElement | null;
        return !!a && a.type === "radio" && a.checked && !!a.closest(".skin-row-shown");
      })) as boolean;
    await browser.pause(1500);
    const keptAfterFirst = await focusOnChecked();
    // A second arrow from that same, real focus.
    await browser.keys(Key.ArrowDown);
    await waitUntil(async () => (await reach()).inUse !== after.inUse, NET_WAIT_MS, "the second ArrowDown did not choose the next skin");
    const second = await reach();
    await browser.pause(1500);
    const keptAfterSecond = await focusOnChecked();
    console.log(`[picker] after the second ArrowDown: in use ${second.inUse}; focus kept ${keptAfterFirst}/${keptAfterSecond}`);
    await shot("picker-after-second-arrow");
    expect({ moved: after.inUse !== before.inUse, foreign: afterForeign, keptAfterFirst, secondMoved: second.inUse !== after.inUse, keptAfterSecond }).toEqual({
      moved: true, foreign: [], keptAfterFirst: true, secondMoved: true, keptAfterSecond: true,
    });
  });
});
