// Plan E7.14 for SK2-N4 (round 24): in every built-in skin, the Host's
// Communities control shows the tree's Communities view with the community
// list, and its Friends control shows Friends again. A knob's stops share one
// box (Combat), so a view behind a knob is reached by pressing the visible
// stop round to it; a control in a pack popup (StealthX's MainMenuPopup rail)
// is pressed in the popup and switches the main window. Each skin is its own
// test. Expected to fail until N4d: StealthX (the main tree shows nothing
// after the popup's switch), total-chaos (its clone sits outside its strip),
// and the art-label packs xslimfire / total-chaos (the clone reads "Files").

import { browser, expect } from "@wdio/globals";
import { shown, switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { applySkin, createIdentity, newUser, showSkinView, skinViewShown, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";

type View = "friends" | "communities";

/** The control for `view`: its box inside its parent's, and its own words. */
async function controlFacts(view: View): Promise<{ inStrip: boolean; text: string } | null> {
  return (await browser.execute((v: string) => {
    const n = Array.from(document.querySelectorAll<HTMLElement>(`[data-skin-role="tab:${v}"]`)).find(
      (e) => e.getBoundingClientRect().width > 0,
    );
    if (!n) return null;
    const r = n.getBoundingClientRect();
    const p = n.parentElement?.getBoundingClientRect();
    const inStrip = !!p && r.left >= p.left - 1 && r.right <= p.right + 1 && r.top >= p.top - 1 && r.bottom <= p.bottom + 1;
    return { inStrip, text: (n.textContent ?? "").trim() };
  }, view)) as { inStrip: boolean; text: string } | null;
}

describe("every skin's Communities control (SK2-N4)", function () {
  this.timeout(600_000);
  const me: User = newUser("n4view");

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  for (const pack of builtinPacks()) {
    it(`${pack.file}: shows Communities, then Friends again`, async () => {
      await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await shown(browser, ".skin-tree");
      await showSkinView(browser, "communities");
      expect(await skinViewShown(browser, "friends")).toBe(false);
      await shown(browser, '[data-skin-role="view:communities"] .buddy-list');
      // The control the Host cloned sits inside its strip and says its own name.
      const facts = await controlFacts("communities");
      if (facts) {
        expect(facts.inStrip).toBe(true);
        expect(facts.text).not.toMatch(/^(files|downloads)$/i);
      }
      await showSkinView(browser, "friends");
      expect(await skinViewShown(browser, "communities")).toBe(false);
    });
  }
});
