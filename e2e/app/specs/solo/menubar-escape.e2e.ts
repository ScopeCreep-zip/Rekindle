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
import { LINKED_SHEET, applySkin, skinShown } from "../../journeys_skins.ts";
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
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, SETTINGS);
      await closeSettings(browser);
      await switchTo(browser, BUDDY);
      // The buddy list re-laid in this pack before any press (a press while
      // the tree is replaced meets the old one: the first run's "combat"
      // pressed Symbiosis's XfireMenu).
      await skinShown(browser, href);
      await browser.pause(1500);

      // Control: no status menu first. What the press meets (every match of
      // the title, what is on top at its centre, the focus and the window's
      // focus), and the trigger's state at once, 100 ms and 1 s after: a menu
      // that never opens, or one that opens and shuts.
      const before = (await browser.execute((sel: string) => {
        const all = Array.from(document.querySelectorAll<HTMLElement>(sel));
        const t = all.find((e) => e.getBoundingClientRect().width > 0);
        const r = t?.getBoundingClientRect();
        const top = r ? document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) : null;
        return JSON.stringify({
          matches: all.length,
          trigger: t ? `${t.tagName.toLowerCase()}.${t.className} node=${t.dataset.skinNode} expanded=${t.getAttribute("aria-expanded")}` : "none",
          topIsTrigger: !!(t && top && t.contains(top)),
          top: top ? `${top.tagName.toLowerCase()}.${(top as HTMLElement).className}` : "none",
          active: document.activeElement ? `${document.activeElement.tagName.toLowerCase()}.${(document.activeElement as HTMLElement).className}` : "none",
          windowFocused: document.hasFocus(),
        });
      }, FILE_MENU)) as string;
      const expanded = async (): Promise<string> =>
        (await browser.execute((sel: string) => `${document.querySelector(sel)?.getAttribute("aria-expanded")}/${document.querySelectorAll('[role="menu"]').length}`, FILE_MENU)) as string;
      await press(browser, await shown(browser, FILE_MENU));
      const at0 = await expanded();
      await browser.pause(100);
      const at100 = await expanded();
      const control = await opens();
      const at1s = await expanded();
      console.log(`[mbesc] ${file} control press: before ${before}; expanded/menus at once ${at0}, 100 ms ${at100}, after wait ${at1s}`);
      if (control) await closeMenu();
      let controlSecond: boolean | null = null;
      if (!control) {
        await press(browser, await shown(browser, FILE_MENU));
        controlSecond = await opens();
        console.log(`[mbesc] ${file} control second press: ${controlSecond}`);
        if (controlSecond) await closeMenu();
      }

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
      // The press met this pack's own tree: the window still shows the skin
      // applied, and the title pressed is the one its served tree carries.
      const sheet = (await browser.execute(LINKED_SHEET)) as string;
      const fileNode = (await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.dataset.skinNode ?? "none", FILE_MENU)) as string;
      console.log(`[mbesc] ${file}: pressed the served title ${fileNode}`);
      expect({ sheet, control, byPress, byEnter }).toEqual({ sheet: href, control: true, byPress: true, byEnter: true });
    });
  }
});
