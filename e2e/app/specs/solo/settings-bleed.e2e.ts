// Lead finding (round 27 review): under faithful skins the buddy list's
// frame art shows in Settings (Combat's camo frame over the skin list,
// Alloy's grey block, Onyx/xSLIMFIRE's stray panel border, Simple-X's short
// panel). For each pack: Settings → Appearance shot, and every rule of the
// skin's sheet (the one `link#rekindle-skin` each window links) that matches
// an element in Settings, logged as `[bleed]` with its selector, the count
// and first element it matches, and its declarations: which rules reach
// Settings, and whether they are the Host's (in the served sheet) or the
// shell's. Symbiosis is the control.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["combat", "alloy", "onyx", "xslimfire", "simple-x", "symbiosis"];

interface Hit {
  selector: string;
  count: number;
  first: string;
  decls: string;
}

/** The skin sheet's style rules matching an element of this document (the Settings window). */
async function skinRulesHere(): Promise<{ href: string; total: number; hits: Hit[]; error: string }> {
  return (await browser.execute(async () => {
    const link = document.getElementById("rekindle-skin") as HTMLLinkElement | null;
    const href = link?.getAttribute("href") ?? "";
    const walk = (rules: CSSRuleList, out: CSSStyleRule[]): void => {
      for (const r of Array.from(rules)) {
        if (r instanceof CSSStyleRule) out.push(r);
        if ("cssRules" in r && (r as CSSGroupingRule).cssRules) walk((r as CSSGroupingRule).cssRules, out);
      }
    };
    const report = (rules: CSSStyleRule[], error: string) => {
      const hits: { selector: string; count: number; first: string; decls: string }[] = [];
      for (const r of rules) {
        let els: Element[] = [];
        try {
          els = Array.from(document.querySelectorAll(r.selectorText));
        } catch {
          continue;
        }
        // :root / html / body rules carry the tokens every window takes.
        els = els.filter((e) => e !== document.documentElement && e !== document.body);
        if (els.length === 0) continue;
        const e = els[0] as HTMLElement;
        const first = `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}.${Array.from(e.classList).join(".")}`;
        hits.push({ selector: r.selectorText.slice(0, 160), count: els.length, first: first.slice(0, 100), decls: r.style.cssText.slice(0, 220) });
      }
      return { href, total: rules.length, hits, error };
    };
    try {
      const out: CSSStyleRule[] = [];
      walk(link!.sheet!.cssRules, out);
      return report(out, "");
    } catch (err) {
      // A cross-origin sheet hides its rules: read its text instead.
      try {
        const text = await (await fetch(href)).text();
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(text);
        const out: CSSStyleRule[] = [];
        walk(sheet.cssRules, out);
        return report(out, `cssRules: ${String(err)} (read by fetch)`);
      } catch (e2) {
        return { href, total: 0, hits: [], error: `${String(err)}; fetch: ${String(e2)}` };
      }
    }
  })) as { href: string; total: number; hits: Hit[]; error: string };
}

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
      const { total, hits, error } = await skinRulesHere();
      console.log(`[bleed] ${file}: ${hits.length} of ${total} skin rules match Settings' elements${error ? ` (${error})` : ""}`);
      for (const h of hits) console.log(`[bleed] ${file}: ${h.selector} -> ${h.count} (${h.first}) { ${h.decls} }`);
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
      expect(total).toBeGreaterThan(0);
    });
  }
});
