// No Xfire branding anywhere the user sees (owner ruling 2026-11-23; plan BR,
// CLAUDE.md "1:1 Xfire look, Rekindle brand"): in every built-in skin, the
// buddy list and every menu its tree opens, and Settings' Appearance and
// About tabs show no "Xfire" in their text, accessible names, tooltips or
// placeholders, link to no xfire.com, and title no window "Xfire". The look
// stays the pack's; only the name and the mark are Rekindle's.
import { browser, expect } from "@wdio/globals";
import { byText, press, shown, sleep, switchTo } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, openSettings } from "../../../walk/nav.ts";
import { applySkin, createIdentity, newUser, skinShown, type User } from "../../journeys.ts";
import { builtinPacks } from "../../skin-pack.ts";

/**
 * Where the window `browser` is in shows Xfire: each finding names the place
 * (visible text, an attribute a user meets, a link target or the window's
 * title) and the words, up to 12.
 */
async function xfireShown(): Promise<string[]> {
  return (await browser.execute(() => {
    const XFIRE = /xfire/i;
    const out: string[] = [];
    const seen = (el: Element): boolean => el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
    if (XFIRE.test(document.title)) out.push(`window title "${document.title}"`);
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const text = (n.textContent ?? "").trim();
      if (text && XFIRE.test(text) && n.parentElement && seen(n.parentElement)) out.push(`text "${text.slice(0, 60)}"`);
    }
    // What a user meets besides the text: a screen reader's name, a tooltip,
    // a placeholder, an image's text and where a link goes.
    for (const el of Array.from(document.body.querySelectorAll("[alt], [title], [aria-label], [placeholder], a[href]"))) {
      for (const attr of ["alt", "title", "aria-label", "placeholder", "href"]) {
        const v = el.getAttribute(attr);
        if (v && XFIRE.test(v)) out.push(`${attr} "${v.slice(0, 60)}" on ${el.tagName.toLowerCase()}`);
      }
    }
    return out.slice(0, 12);
  })) as string[];
}

/**
 * The buddy list's logo nodes and branded decor (plan BR) that do not draw
 * the Rekindle mark in their box: each must hold a visible brand glyph
 * (`.skin-node-glyph-brand`) inside its own box, over the pack's art.
 */
async function logosUnbranded(): Promise<string[]> {
  return (await browser.execute(() => {
    const DECOR = ["TheX", "TheXCover", "InfoLogo", "Logo2"];
    const nodes = Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-node]")).filter(
      (el) => el.dataset.skinRole === "logo" || DECOR.includes(el.dataset.skinNode ?? ""),
    );
    return nodes.flatMap((el) => {
      if (!el.checkVisibility()) return [];
      const id = el.dataset.skinNode ?? "?";
      const mark = el.querySelector<SVGElement>(":scope > .skin-node-label .skin-node-glyph-brand");
      if (!mark || !mark.checkVisibility()) return [`${id} draws no Rekindle mark`];
      const n = el.getBoundingClientRect();
      const m = mark.getBoundingClientRect();
      const inside = m.width > 0 && m.height > 0 && m.left >= n.left - 0.5 && m.top >= n.top - 0.5 && m.right <= n.right + 0.5 && m.bottom <= n.bottom + 0.5;
      return inside ? [] : [`${id}'s mark lies outside its box`];
    });
  })) as string[];
}

/** Each menu the buddy list's tree opens (a pack's menu titles), by its served action, with its items' Xfire words. */
async function menusShowXfire(): Promise<string[]> {
  const out: string[] = [];
  const actions = (await browser.execute(() =>
    Array.from(document.querySelectorAll<HTMLElement>(".skin-tree [data-skin-action^='menu:']"))
      .filter((el) => el.checkVisibility())
      .map((el) => el.getAttribute("data-skin-action") ?? ""),
  )) as string[];
  for (const action of [...new Set(actions)]) {
    await press(browser, await shown(browser, `.skin-tree [data-skin-action="${action}"]`));
    await shown(browser, ".menu-dropdown");
    for (const found of await xfireShown()) out.push(`${action}: ${found}`);
    await browser.keys("Escape");
    await sleep(300);
  }
  return out;
}

describe("no Xfire branding where the user sees", function () {
  this.timeout(900_000);
  const me: User = newUser("branding");

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
  });

  it("Settings' About and Appearance tabs say Rekindle, never Xfire", async () => {
    await switchTo(browser, SETTINGS);
    await (await byText(browser, ".form-tab", "About")).click();
    await shown(browser, ".settings-about-text");
    expect(await xfireShown()).toEqual([]);
    await (await byText(browser, ".form-tab", "Appearance")).click();
    await shown(browser, ".skin-row .skin-name");
    expect(await xfireShown()).toEqual([]);
  });

  for (const pack of builtinPacks()) {
    it(`${pack.file}: the buddy list and its menus show no Xfire`, async () => {
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      const found = [...(await xfireShown()), ...(await logosUnbranded()), ...(await menusShowXfire())];
      expect(found).toEqual([]);
    });
  }
});
