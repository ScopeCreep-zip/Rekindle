// Lead finding (round 27 review): under faithful skins the buddy list's
// frame art shows in Settings (Combat's camo frame over the skin list,
// Alloy's grey block, Onyx/xSLIMFIRE's stray panel border, Simple-X's short
// panel). For each pack: Settings → Appearance shot, and every rule of the
// skin's sheet (the one `link#rekindle-skin` each window links) that matches
// an element in Settings, logged as `[bleed]` with its selector, the count
// and first element it matches, and its declarations: which rules reach
// Settings, and whether they are the Host's (in the served sheet) or the
// shell's. Symbiosis is the control. The sheet is cross-origin to the page,
// so its rules are read through computed styles: each element drawn with
// skin:// art. Plan SB1 makes it a proof: Settings (data-window="settings")
// holds no element drawn with pack art, and the buddy list still does.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { artElements, windowKind } from "../../skin-art.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["combat", "alloy", "onyx", "xslimfire", "simple-x", "symbiosis"];

describe("Settings under faithful skins: what of the skin's sheet reaches it", function () {
  this.timeout(1_800_000);
  const me: User = newUser("bleed");

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  for (const file of PACKS) {
    it(`${file}: Settings → Appearance, and the skin rules matching Settings' elements`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      const href = await applySkin(browser, me, pack.name);
      await skinShown(browser, href);
      await browser.pause(1500);
      await shot("settings-appearance");
      // Plan SB1: the window says its kind, and no pack art reaches Settings
      // (no pack defines it); theme tokens only.
      const kind = await windowKind();
      const painted = await artElements();
      console.log(`[bleed] ${file}: ${painted.length} element(s) in Settings drawn with skin art`);
      for (const line of painted.slice(0, 25)) console.log(`[bleed] ${file}: ${line.slice(0, 400)}`);
      // The element under the skin list's middle: what paints over it.
      const over = (await browser.execute(() => {
        const list = document.querySelector(".skin-list, .skin-row")?.closest("div") as HTMLElement | null;
        const r = (list ?? document.body).getBoundingClientRect();
        const el = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(r.height / 2, 120)) as HTMLElement | null;
        if (!el) return "nothing";
        const cs = getComputedStyle(el);
        return `${el.tagName.toLowerCase()}.${Array.from(el.classList).join(".")} bg=${cs.backgroundImage.slice(0, 80)} ${cs.backgroundColor} border=${cs.borderTopWidth} ${cs.borderTopStyle}`;
      })) as string;
      console.log(`[bleed] ${file}: at the skin list's middle: ${over}`);
      await switchTo(browser, BUDDY);
      await shot("buddy-list");
      // The buddy list is the pack's own window: its art is unchanged.
      const buddyArt = await artElements();
      console.log(`[bleed] ${file}: ${buddyArt.length} element(s) in the buddy list drawn with skin art`);
      await openSettings(browser, "Appearance");
      expect(kind).toBe("settings");
      expect(painted).toEqual([]);
      expect(buddyArt.length).toBeGreaterThan(0);
    });
  }
});
