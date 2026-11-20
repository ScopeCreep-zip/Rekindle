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
import { WAIT_MS, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { violations } from "../../axe.ts";
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
  async function audit(pack: SkinPack, w: WindowCase, withData: boolean, allow: string[] = []): Promise<Record<string, unknown>> {
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
        allow,
        overlapAllow: w.overlapAllow,
      });
      for (const [check, value] of Object.entries(geometry)) found[`${where}: ${check}`] = value;
    }
    return found;
  }

  /**
   * Settings is opened from the buddy list's menubar, which returns focus
   * to its trigger when the menu closes (WAI-ARIA menubar pattern). Kobalte
   * then keeps that trigger highlighted (`data-highlighted`,
   * `data-pointer-type="mouse"`) until a real pointer leaves it, and the
   * driver's synthetic pointer never does: a dispatched `pointerleave`
   * does not clear it (probes 2026-11-20). So the focus check would read
   * the trigger already showing its highlight and report "no indicator",
   * and a focus moved elsewhere at once is taken back. Wait until focus
   * stays put, then blur; the focus check passes over the trigger only
   * while it carries that highlight (`MENUBAR_HELD`), and checks every
   * other control.
   */
  async function leaveMenubar(): Promise<void> {
    await waitUntil(
      async () =>
        (await browser.execute(`if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
return new Promise(function (done) { setTimeout(function () { done(document.activeElement === document.body); }, 200); });`)) as boolean,
      WAIT_MS,
      "focus kept returning to the menubar trigger",
    );
  }

  /** The menubar trigger the driver's pointer left highlighted (`leaveMenubar`). */
  const MENUBAR_HELD = ["button.menu-bar-item[data-highlighted]"];

  for (const pack of PACKS) {
    it(`${pack.name}: axe and geometry hold in the buddy list, Appearance and a community window`, async () => {
      await applySkin(browser, me, pack.name);
      const found: Record<string, unknown> = {};
      Object.assign(found, await audit(pack, SETTINGS_APPEARANCE, true));
      await switchTo(browser, BUDDY);
      await leaveMenubar();
      Object.assign(found, await audit(pack, BUDDY_LIST, false, MENUBAR_HELD));
      await browser.switchToWindow(communityHandle);
      Object.assign(found, await audit(pack, COMMUNITY, true));
      expect(found).toEqual({});
    });
  }
});
