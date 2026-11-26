// Lead (2026-11-26): after the status menu is closed with Escape, does the
// user's next press on another menu title open it? Route-paused's Combat
// step found no Settings window after Symbiosis's status menu. Each case is
// measured, with a control, by the input a user gives: a press (pointerdown,
// mousedown, pointerup, mouseup, click; Kobalte's triggers open on
// pointerdown, so the driver's element click, a JS click, opens none) and
// the keyboard (Enter, then ArrowDown, on the focused title).

import { browser, expect } from "@wdio/globals";
import { Key } from "webdriverio";
import { gone, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, closeSettings, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const STATUS = '.skin-tree [data-skin-role="statusList"]';
const FILE_MENU = '[data-skin-action="menu:file"]';

/** Whether a menu is open within `ms`. */
async function opens(ms = 5_000): Promise<boolean> {
  return waitUntil(async () => (await browser.$('[role="menu"]').isDisplayed().catch(() => false)), ms, "").then(() => true, () => false);
}

async function closeMenu(): Promise<void> {
  await browser.keys(Key.Escape);
  await gone(browser, '[role="menu"]');
}

/** Open the status menu and close it with Escape, as a user does. */
async function statusThenEscape(): Promise<void> {
  await press(browser, await shown(browser, STATUS));
  if (!(await opens())) throw new Error("the status menu did not open");
  await closeMenu();
}

describe("after Escape closes the status menu, the next menu title opens (lead)", function () {
  this.timeout(900_000);
  const me: User = { ...newUser("mbesc"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
  });

  for (const file of ["symbiosis", "combat"]) {
    it(`${file}: Rekindle opens by press and by keyboard after the status menu's Escape`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await openSettings(browser, "Appearance");
      await applySkin(browser, me, pack.name);
      await switchTo(browser, SETTINGS);
      await closeSettings(browser);
      await switchTo(browser, BUDDY);

      // Control: no status menu first.
      await press(browser, await shown(browser, FILE_MENU));
      const control = await opens();
      if (control) await closeMenu();

      // A press right after the status menu's Escape.
      await statusThenEscape();
      const active = (await browser.execute(() => {
        const a = document.activeElement as HTMLElement | null;
        return a ? `${a.tagName.toLowerCase()}.${a.className} ${a.dataset.skinNode ?? ""}` : "none";
      })) as string;
      await press(browser, await shown(browser, FILE_MENU));
      const byPress = await opens();
      await shot(`${file}-after-escape-press`);
      if (byPress) await closeMenu();
      // A second press, when the first opened nothing.
      let secondPress: boolean | null = null;
      if (!byPress) {
        await press(browser, await shown(browser, FILE_MENU));
        secondPress = await opens();
        if (secondPress) await closeMenu();
      }

      // The keyboard: focus on the title, Enter, then ArrowDown.
      await statusThenEscape();
      await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.focus(), FILE_MENU);
      await browser.keys(Key.Enter);
      const byEnter = await opens();
      if (byEnter) await closeMenu();
      let byArrow: boolean | null = null;
      if (!byEnter) {
        await browser.keys(Key.ArrowDown);
        byArrow = await opens();
        if (byArrow) await closeMenu();
      }
      console.log(`[mbesc] ${file}: control ${control}; after Escape (focus on ${active}): press ${byPress}, second press ${secondPress}; keyboard Enter ${byEnter}, ArrowDown ${byArrow}`);
      expect({ control, byPress, byEnter }).toEqual({ control: true, byPress: true, byEnter: true });
    });
  }
});
