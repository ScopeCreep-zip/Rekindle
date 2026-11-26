// Lead (CR1 audit, priority): double-clicking a friend opens the chat in
// every pack, and a friend row keeps its element across a signal push.
// Selecting a row pushes the window's selection, so the Host pushes a new
// signal set; when the friends list re-created its rows on every change,
// a double-click straddling that push landed on a replaced row and no chat
// opened (the live-signals branch's parity setup). Per pack: Cara's row is
// tagged with a test expando, selected, and after the push must be the same
// element; then a double-click opens her chat window, which is closed.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, switchTo, waitForHandle } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, friendRow, newUser, openChat, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const PACKS = ["symbiosis", "combat", "onyx", "alloy", "xslimfire", "simple-x", "total-chaos", "xfire-classic"];

describe("a friend row survives a signal push, and a double-click opens the chat (CR1 audit)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("rowstable"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    await befriendFleet(browser, me, peers);
  });

  after(async () => {
    await stopFleet(peers);
  });

  for (const file of PACKS) {
    it(`${file}: Cara's row keeps its element across the selection's signal push, and a double-click opens her chat`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      const row = await friendRow(browser, "Cara");
      await browser.execute((el: HTMLElement) => {
        (el as HTMLElement & { __rowOnce?: number }).__rowOnce = 1;
      }, row as unknown as HTMLElement);
      await row.click();
      await browser.pause(2000);
      const kept = (await browser.execute(() => {
        const r = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((e) => (e.textContent ?? "").includes("Cara"));
        return { same: (r as (HTMLElement & { __rowOnce?: number }) | undefined)?.__rowOnce === 1, signals: document.querySelector<HTMLElement>(".skin-tree")?.dataset.skinSignals ?? "no tree" };
      })) as { same: boolean; signals: string };
      console.log(`[row] ${file}: after selecting Cara: ${JSON.stringify(kept)}`);
      await shot(`${file}-row-selected`);
      expect({ pack: file, rowKept: kept.same }).toEqual({ pack: file, rowKept: true });
      await openChat(browser, "Cara");
      const chat = await browser.getWindowHandle();
      console.log(`[row] ${file}: double-click opened ${chat}`);
      await shot(`${file}-chat-opened`);
      await browser.closeWindow();
      await switchTo(browser, BUDDY);
      await waitForHandle(browser, (h) => h === BUDDY, NET_WAIT_MS);
    });
  }
});
