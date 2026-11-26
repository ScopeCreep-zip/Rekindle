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
import { BUDDY, SETTINGS, closeSettings, openSettings } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

/** Every pack the skin round ships. Each serves the community window's
 *  slots (xfire-classic and Total Chaos too, though they have no chat
 *  golden); a pack that served none would be said so from what is drawn. */
const PACKS = ["symbiosis", "alloy", "xslimfire", "combat", "onyx", "simple-x", "xfire-classic", "total-chaos"];

type Box = [number, number, number, number];

/** The boxes the proof compares: each slot's, its mounted part's, the layout's and the composer's. */
async function slotBoxes(): Promise<{ area: Box | null; layout: Box | null; edit: Box | null; composer: Box | null; send: Box | null; sendPart: Box | null; field: Box | null; controls: Box[]; view: Box }> {
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
      // The composer's text field and every control it draws (attach,
      // mic, emoji, a shell Send): a usable composer has a field a line
      // tall and all of them in the window.
      field: box(document.querySelector(".message-input-field")),
      controls: Array.from(document.querySelectorAll(".message-input-shell > *"))
        .filter((e) => e.getBoundingClientRect().width > 0)
        .map((e) => box(e) as [number, number, number, number]),
      view: [0, 0, window.innerWidth, window.innerHeight] as [number, number, number, number],
    };
  })) as { area: Box | null; layout: Box | null; edit: Box | null; composer: Box | null; send: Box | null; sendPart: Box | null; field: Box | null; controls: Box[]; view: Box };
}

/** A text field a line tall: 12 px text in 6 px padding (`.message-input`) and its border. */
const MIN_FIELD_PX = 24;

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
      // Settings closes after each switch, so the next pack's Settings
      // opens fresh and frontmost (left open behind the kept community
      // window, its Appearance tab once took no click).
      await switchTo(browser, SETTINGS);
      await closeSettings(browser);
      await browser.switchToWindow(community);
      await skinShown(browser, href);
      await browser.pause(2000);
      console.log(`[slots] ${file}: ${await slotEvidence()}`);
      await shot(`${file}-community`);
      const b = await slotBoxes();
      console.log(`[slots-proof] ${file}: ${JSON.stringify(b)}`);
      if (b.area === null && b.edit === null) {
        // No chat frame served: the community window is the app's own
        // layout, whole and on screen, with its composer inside it.
        const view = (await browser.execute(() => [0, 0, window.innerWidth, window.innerHeight])) as [number, number, number, number];
        console.log(`[slots-proof] ${file}: no chat frame served (no chatArea/chatEdit slot drawn); window ${JSON.stringify(view)}`);
        expect({ pack: file, layout: b.layout !== null, composerOnScreen: within(b.composer, view) }).toEqual({ pack: file, layout: true, composerOnScreen: true });
        return;
      }
      expect({ pack: file, layoutFillsChatArea: near(b.layout, b.area) }).toEqual({ pack: file, layoutFillsChatArea: true });
      // Within the slot is not enough: a 2 px slot holds a crushed composer.
      const fieldHeight = b.field ? b.field[3] - b.field[1] : 0;
      const outside = b.controls.filter((c) => !within(c, b.view)).length;
      expect({ pack: file, fieldAtLeastALine: fieldHeight >= MIN_FIELD_PX, controlsOutsideWindow: outside }).toEqual({ pack: file, fieldAtLeastALine: true, controlsOutsideWindow: 0 });
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
    await switchTo(browser, SETTINGS);
    await closeSettings(browser);
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
    await switchTo(browser, SETTINGS);
    await closeSettings(browser);
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
