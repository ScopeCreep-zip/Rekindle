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
import { applySkin, createIdentity, newUser, showSkinView, skinShown, skinViewShown, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";

type View = "friends" | "communities";

/** The control for `view`: its box inside its parent's, and its own words. */
async function controlFacts(view: View): Promise<{ inStrip: boolean; text: string; art: number[]; strip: number[] } | null> {
  return (await browser.execute(async (v: string) => {
    const n = Array.from(document.querySelectorAll<HTMLElement>(`[data-skin-role="tab:${v}"]`)).find(
      (e) => e.getBoundingClientRect().width > 0,
    );
    if (!n) return null;
    // Its art box, not its box: SK7 grows a small control's box to a 24 px
    // target about its art, past a 16 px strip. The served target names the
    // art's rect inside that box (NodeTarget.art [x, y, w, h]).
    type Node = { id: string; target?: { art: [number, number, number, number] }; children?: Node[] };
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke(c: string, a: unknown): Promise<{ value: { shells?: { root: Node[] }[] } }> } };
    let art: [number, number, number, number] | undefined;
    const find = (x: Node): void => {
      if (x.id === n.dataset.skinNode) art = x.target?.art;
      (x.children ?? []).forEach(find);
    };
    for (const sh of (await w.__TAURI_INTERNALS__.invoke("get_skin", { id: null })).value.shells ?? []) sh.root.forEach(find);
    const b = n.getBoundingClientRect();
    const r = art
      ? { left: b.left + art[0], top: b.top + art[1], right: b.left + art[0] + art[2], bottom: b.top + art[1] + art[3] }
      : { left: b.left, top: b.top, right: b.right, bottom: b.bottom };
    const p = n.parentElement?.getBoundingClientRect();
    const inStrip = !!p && r.left >= p.left - 1 && r.right <= p.right + 1 && r.top >= p.top - 1 && r.bottom <= p.bottom + 1;
    const round = (v: number): number => Math.round(v * 10) / 10;
    return {
      inStrip,
      text: (n.textContent ?? "").trim(),
      art: [r.left, r.top, r.right, r.bottom].map(round),
      strip: p ? [p.left, p.top, p.right, p.bottom].map(round) : [],
    };
  }, view)) as { inStrip: boolean; text: string; art: number[]; strip: number[] } | null;
}

/**
 * Whether the element at the centre of the shown Friends tab or knob (where
 * a user clicks) is inside a node of that control listening for a press. A knob shared by
 * every stop (Combat) shows no Friends knob on another view: true then, as
 * the press there is the shown knob's own.
 */
async function friendsHitListens(): Promise<boolean> {
  return (await browser.execute(() => {
    const control = Array.from(document.querySelectorAll<HTMLElement>('[data-skin-role="tab:friends"], [data-skin-role="knob:friends"]')).find(
      (n) => n.getBoundingClientRect().width > 0,
    );
    if (!control) return true;
    const r = control.getBoundingClientRect();
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    const listener = hit?.closest("[data-skin-on]");
    // The control's own press (itself or a part of it), not a neighbour's or
    // a resize grip laid over it (Onyx's TopLeft grip over its Friends tab).
    return !!listener && control.contains(listener) && /\b(leftDown|click)\b/.test(listener.getAttribute("data-skin-on") ?? "");
  })) as boolean;
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
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      // Its tree is the new skin's only once it shows the new sheet.
      await skinShown(browser, href);
      await shown(browser, ".skin-tree");
      await showSkinView(browser, "communities");
      expect(await skinViewShown(browser, "friends")).toBe(false);
      await shown(browser, '[data-skin-role="view:communities"] .buddy-list');
      // The control the Host cloned sits inside its strip and says its own name.
      const facts = await controlFacts("communities");
      if (facts) {
        // Its art box [left, top, right, bottom] inside its strip's, named when not.
        if (!facts.inStrip) throw new Error(`the cloned Communities tab lies outside its strip: art ${JSON.stringify(facts.art)}, strip ${JSON.stringify(facts.strip)}`);
        expect(facts.text).not.toMatch(/^(files|downloads)$/i);
      }
      // A user's click on the pack's Friends control reaches a node that
      // listens for presses (no text or decor laid over it takes the press:
      // Xfire Classic's tab text over its button, r24 896e58096).
      expect(await friendsHitListens()).toBe(true);
      await showSkinView(browser, "friends");
      expect(await skinViewShown(browser, "communities")).toBe(false);
    });
  }
});
