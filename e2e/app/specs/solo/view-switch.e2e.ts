// Round 34 (lead): round 33's radio rule had dropped real page selections
// (Onyx's Servers tab `GroupMessage(1)`, Combat's knob `GroupMessage(2)`)
// once every pack was re-imported. For Onyx the Servers tab, and for Combat
// the knob turned through Servers, Files and back to Friends: at each stop
// the view is drawn and visible and the other views are not, each logged as
// `[view]` and shot.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, showSkinView, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const CASES = [
  { file: "onyx", stops: ["servers", "friends"] },
  { file: "combat", stops: ["servers", "files", "friends"] },
] as const;

/** Every view node in the window, and whether it is drawn and visible. */
async function views(): Promise<Record<string, boolean>> {
  return (await browser.execute(() => {
    const out: Record<string, boolean> = {};
    for (const n of Array.from(document.querySelectorAll<HTMLElement>('[data-skin-role^="view:"]'))) {
      const key = (n.dataset.skinRole ?? "").slice("view:".length);
      const r = n.getBoundingClientRect();
      out[key] = out[key] || (n.closest("[hidden]") === null && r.width > 0 && r.height > 0);
    }
    return out;
  })) as Record<string, boolean>;
}

describe("a pack's own view controls switch its views (round 34: Onyx's Servers tab, Combat's knob)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("viewsw"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
  });

  for (const { file, stops } of CASES) {
    it(`${file}: ${stops.join(" → ")}, each the one view shown`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      for (const stop of stops) {
        await showSkinView(browser, stop);
        await browser.pause(800);
        const shown = await views();
        console.log(`[view] ${file} → ${stop}: ${JSON.stringify(shown)}`);
        await shot(`${file}-${stop}`);
        expect(shown[stop]).toBe(true);
        expect(Object.entries(shown).filter(([k, v]) => v && k !== stop).map(([k]) => k)).toEqual([]);
      }
    });
  }
});
