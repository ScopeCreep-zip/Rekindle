// Plan SK17-7a: in every skin, the Community window draws its icons with the
// shell's own glyphs (SkinGlyph: SVG in the role's colour, or the pack's
// served image), never the icon font. With a text channel, a voice channel
// (not joined: nothing opens the microphone) and our own posted message (its
// status icon), each skin's Community window must hold no `.nf-icon`, no
// visible text in the icon font, and every glyph must draw.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, resolveIntent, shown, switchTo, typeInto } from "../../../walk/helpers.ts";
import { BUDDY, openSettings, setValue } from "../../../walk/nav.ts";
import { applySkin, createCommunity, createIdentity, newUser, openCommunity, postMessage, skinShown, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";

/** What in the window `browser` is in still uses the icon font, or a glyph that draws nothing. */
async function glyphProblems(): Promise<string[]> {
  return (await browser.execute(() => {
    const out: string[] = [];
    const root = document.querySelector(".app-frame") ?? document.body;
    const left = root.querySelectorAll(".nf-icon");
    if (left.length > 0) out.push(`${left.length} .nf-icon left (${Array.from(left).slice(0, 3).map((e) => e.className).join(" | ")})`);
    // Visible text drawn in the icon font, whatever its class.
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const el = n.parentElement;
      if (!el || !(n.textContent ?? "").trim() || !el.checkVisibility()) continue;
      if (/nerd/i.test(getComputedStyle(el).fontFamily)) out.push(`icon-font text in ${el.className || el.tagName}`);
    }
    for (const g of Array.from(root.querySelectorAll<HTMLElement>(".skin-glyph"))) {
      if (!g.checkVisibility()) continue;
      const r = g.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) out.push(`${g.dataset.glyph} draws in an empty box`);
      else if (g.classList.contains("skin-glyph-served")) {
        if (getComputedStyle(g).backgroundImage === "none") out.push(`${g.dataset.glyph} served but no image`);
      } else if (!g.querySelector("svg path")?.getAttribute("d")) out.push(`${g.dataset.glyph} has no path`);
    }
    return out.slice(0, 12);
  })) as string[];
}

describe("the Community window draws the shell's glyphs in every skin", function () {
  this.timeout(1_200_000);
  const me: User = newUser("glyphs");
  let community = "";

  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Glyph Check");
    await openCommunity(browser, "Glyph Check");
    community = await browser.getWindowHandle();
    // A voice channel, as the walk makes one (rows-voice.ts M10v); not joined.
    await (await shown(browser, '[title="Create Channel"]')).click();
    await typeInto(await shown(browser, 'input[placeholder="Channel name..."]'), "Lounge");
    await setValue(browser, ".modal-container select.form-select", "voice");
    await (await browser.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await gone(browser, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
    await postMessage(browser, me, "glyphs, everyone");
    await openSettings(browser, "Appearance");
  });

  for (const pack of builtinPacks()) {
    it(`${pack.file}: no icon-font glyph, every glyph drawn`, async () => {
      const href = await applySkin(browser, me, pack.name);
      await browser.switchToWindow(community);
      await skinShown(browser, href);
      await shown(browser, ".skin-glyph");
      expect(await glyphProblems()).toEqual([]);
      await switchTo(browser, BUDDY);
    });
  }
});
