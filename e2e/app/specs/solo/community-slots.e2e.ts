// Lead (SKW2 split): in a skinned community window, which slots the pack's
// composed tree lays and how the community content fills them. Logs, for
// Symbiosis (content stops ~400 px down) and Alloy (no content drawn): every
// slot node's box, each child the shell mounted in it (Solid's Portal
// wrapper) with its box and display, the community layout's box and its
// parents up to the slot, and the window's size; shot.
//
// Then the slots slice's proof (lead): the community layout fills its
// chatArea slot (its box equals the slot's, ±1: it stopped at its content
// height, 337 of 449 px, in an unsized Portal wrapper), the composer lies
// within chatEdit (it overflowed by 14 px), and the shell's part fills
// chatSend.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["symbiosis", "alloy", "xslimfire"];

type Box = [number, number, number, number];

/** The boxes the proof compares: each slot's, its mounted part's, the layout's and the composer's. */
async function slotBoxes(): Promise<{ area: Box | null; layout: Box | null; edit: Box | null; composer: Box | null; send: Box | null; sendPart: Box | null }> {
  return (await browser.execute(() => {
    const box = (e: Element | null | undefined): [number, number, number, number] | null => {
      if (!e) return null;
      const r = e.getBoundingClientRect();
      return [r.left, r.top, r.right, r.bottom];
    };
    const slot = (k: string) => document.querySelector(`.skin-tree [data-skin-role="slot:${k}"]`);
    return {
      area: box(slot("chatArea")),
      layout: box(document.querySelector(".community-layout")),
      edit: box(slot("chatEdit")),
      composer: box(document.querySelector(".message-input-wrapper")),
      send: box(slot("chatSend")),
      sendPart: box(slot("chatSend")?.firstElementChild),
    };
  })) as { area: Box | null; layout: Box | null; edit: Box | null; composer: Box | null; send: Box | null; sendPart: Box | null };
}

const near = (a: Box | null, b: Box | null): boolean => !!a && !!b && a.every((v, i) => Math.abs(v - b[i]) <= 1);
const within = (a: Box | null, b: Box | null): boolean => !!a && !!b && a[0] >= b[0] - 1 && a[1] >= b[1] - 1 && a[2] <= b[2] + 1 && a[3] <= b[3] + 1;

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

  // The community window opens once and stays open: a skin switch re-skins
  // it in place. Re-opening it after a close waited for a new handle that
  // never came (the window keeps its label, so its handle is not new).
  let community = "";
  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Slot Hall");
    await switchTo(browser, BUDDY);
    await openCommunity(browser, "Slot Hall");
    community = await browser.getWindowHandle();
  });

  for (const file of PACKS) {
    it(`${file}: the community window's slot boxes and mounted content`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await browser.switchToWindow(community);
      await skinShown(browser, href);
      await browser.pause(2000);
      console.log(`[slots] ${file}: ${await slotEvidence()}`);
      await shot(`${file}-community`);
      const b = await slotBoxes();
      console.log(`[slots-proof] ${file}: ${JSON.stringify(b)}`);
      expect({ pack: file, layoutFillsChatArea: near(b.layout, b.area) }).toEqual({ pack: file, layoutFillsChatArea: true });
      expect({ pack: file, composerWithinChatEdit: within(b.composer, b.edit) }).toEqual({ pack: file, composerWithinChatEdit: true });
      if (b.send) expect({ pack: file, partFillsChatSend: near(b.sendPart, b.send) }).toEqual({ pack: file, partFillsChatSend: true });
    });
  }

  // Lead (CR1 audit fix 2): a slot's part is created once. A draft typed in
  // the community composer survives a skin switch, which re-creates the
  // window's tree and re-registers its slots: the composer is the same
  // element (an expando the test sets stays on it) with the same text.
  it("a skin switch keeps the composer's element and its draft", async () => {
    const [from, to] = ["symbiosis", "alloy"].map((f) => builtinPacks().find((p) => p.file === f));
    if (!from || !to) throw new Error("no built-in symbiosis/alloy");
    await switchTo(browser, BUDDY);
    await openSettings(browser, "Appearance");
    const fromHref = await applySkin(browser, me, from.name);
    await browser.switchToWindow(community);
    await skinShown(browser, fromHref);
    const handle = community;
    const draft = `draft ${Date.now() % 100000}`;
    const field = await browser.$(".message-input-field");
    await field.click();
    await browser.keys(draft.split(""));
    await browser.execute(() => {
      const f = document.querySelector(".message-input-field") as (HTMLElement & { __slotOnce?: number }) | null;
      if (f) f.__slotOnce = 1;
    });
    await switchTo(browser, BUDDY);
    await openSettings(browser, "Appearance");
    const href = await applySkin(browser, me, to.name);
    await browser.switchToWindow(handle);
    await skinShown(browser, href);
    await browser.pause(1500);
    const after = (await browser.execute(() => {
      const f = document.querySelector(".message-input-field") as (HTMLTextAreaElement & { __slotOnce?: number }) | null;
      return { same: f?.__slotOnce === 1, text: f ? (f.value ?? f.textContent ?? "") : null, count: document.querySelectorAll(".message-input-field").length };
    })) as { same: boolean; text: string | null; count: number };
    console.log(`[slots-once] after the switch: ${JSON.stringify(after)} (draft "${draft}")`);
    await shot("draft-after-switch");
    expect(after).toEqual({ same: true, text: draft, count: 1 });
  });
});
