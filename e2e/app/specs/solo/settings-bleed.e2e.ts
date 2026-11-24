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
// skin:// art.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
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
      // The skin sheet is cross-origin to the page (its rules are not
      // readable), so what it paints is read from computed styles: every
      // element in Settings drawn with skin:// art, its classes, ancestors
      // and the art's name.
      const painted = (await browser.execute(() => {
        const name = (el: Element): string => `${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}${el.classList.length ? "." + Array.from(el.classList).join(".") : ""}`;
        const out: string[] = [];
        for (const el of Array.from(document.querySelectorAll<HTMLElement>("body *"))) {
          const cs = getComputedStyle(el);
          const art = [cs.backgroundImage, cs.borderImageSource].filter((v) => v && v.includes("skin://"));
          if (art.length === 0) continue;
          const path: string[] = [];
          for (let p = el.parentElement; p && p !== document.body && path.length < 5; p = p.parentElement) path.push(name(p));
          const r = el.getBoundingClientRect();
          const file = art.map((a) => (a.match(/skin:\/\/[^)"]+/g) ?? []).map((u) => u.split("/").slice(-2).join("/")).join(",")).join(" ");
          out.push(`${name(el)} [${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}] art=${file.slice(0, 120)} in ${path.reverse().join(" > ")}`);
        }
        return out;
      })) as string[];
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
      await openSettings(browser, "Appearance");
      expect(painted).toBeDefined();
    });
  }
});
