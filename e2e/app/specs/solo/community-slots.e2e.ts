// Lead (SKW2 split): in a skinned community window, which slots the pack's
// composed tree lays and how the community content fills them. Logs, for
// Symbiosis (content stops ~400 px down) and Alloy (no content drawn): every
// slot node's box, each child the shell mounted in it (Solid's Portal
// wrapper) with its box and display, the community layout's box and its
// parents up to the slot, and the window's size; shot.

import { browser } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["symbiosis", "alloy"];

async function slotEvidence(): Promise<string> {
  return (await browser.execute(() => {
    const box = (e: Element): string => {
      const r = e.getBoundingClientRect();
      return `${Math.round(r.x)},${Math.round(r.y)} ${Math.round(r.width)}x${Math.round(r.height)}`;
    };
    const desc = (e: Element): string => {
      const cs = getComputedStyle(e);
      return `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || "-"} ${box(e)} display=${cs.display} dir=${cs.flexDirection} flex=${cs.flex} h=${cs.height} overflow=${cs.overflow}`;
    };
    const slots = Array.from(document.querySelectorAll<HTMLElement>('.skin-tree [data-skin-role^="slot:"]')).map((s) => ({
      slot: `${s.dataset.skinRole} ${s.dataset.skinNode} ${desc(s)}`,
      mounted: Array.from(s.children).map(desc),
    }));
    const layout = document.querySelector(".community-layout");
    const chain: string[] = [];
    for (let e: Element | null = layout; e && !e.matches(".skin-tree"); e = e.parentElement) chain.push(desc(e));
    return JSON.stringify({ window: [innerWidth, innerHeight], tree: document.querySelector(".skin-tree") ? box(document.querySelector(".skin-tree")!) : "none", slots, layoutChain: chain, composer: document.querySelector(".message-input-field") ? box(document.querySelector(".message-input-field")!) : "none" });
  })) as string;
}

describe("a skinned community window's slots and the content mounted in them (SKW2 split)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("cslots"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Slot Hall");
  });

  for (const file of PACKS) {
    it(`${file}: the community window's slot boxes and mounted content`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await openCommunity(browser, "Slot Hall");
      await skinShown(browser, href);
      await browser.pause(2000);
      console.log(`[slots] ${file}: ${await slotEvidence()}`);
      await shot(`${file}-community`);
      await browser.closeWindow();
      await switchTo(browser, BUDDY);
    });
  }
});
