// Lead (CR1 audit, priority): double-clicking a friend opens the chat in
// every pack, and a friend row keeps its element across a signal push.
// Selecting a row pushes the window's selection, so the Host pushes a new
// signal set; when the friends list re-created its rows on every change,
// a double-click straddling that push landed on a replaced row and no chat
// opened (the live-signals branch's parity setup). Per pack: Cara's row is
// tagged with a test expando, selected, and after the push must be the same
// element; then a double-click opens her chat window, which is closed.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, closeSettings, openSettings } from "../../../walk/nav.ts";
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
      // Settings closes after each switch, so the next pack's opens fresh
      // and frontmost (left open behind the chat window this test opens,
      // its Appearance tab took no click for Alloy).
      await switchTo(browser, SETTINGS);
      await closeSettings(browser);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      // Cara appears offline first, so her presence moves her from the
      // group's Offline section to its Online one after the click (lead:
      // the move must keep her row's element).
      const sectionNow = async (): Promise<string> =>
        (await browser.execute(() => {
          const r = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((e) => (e.textContent ?? "").includes("Cara"));
          return r?.closest<HTMLElement>("[data-buddy-section]")?.dataset.buddySection ?? "none";
        })) as string;
      await peers[0].cli(["presence", "set", "--status", "invisible"]);
      await waitUntil(async () => (await sectionNow()) === "Offline", NET_WAIT_MS, "Cara never showed offline");
      const row = await friendRow(browser, "Cara");
      // Evidence (lead, Symbiosis's replaced row): Cara's section (the
      // group's Online or Offline list) and status before the click and
      // after the push, and every Cara row added or removed meanwhile.
      const where = async (): Promise<string> =>
        (await browser.execute(() => {
          const r = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((e) => (e.textContent ?? "").includes("Cara"));
          return JSON.stringify({
            section: r?.closest<HTMLElement>("[data-buddy-section]")?.dataset.buddySection ?? "none",
            status: r?.querySelector("[role=img]")?.getAttribute("aria-label") ?? "none",
          });
        })) as string;
      await browser.execute((el: HTMLElement) => {
        (el as HTMLElement & { __rowOnce?: number }).__rowOnce = 1;
        const w = window as unknown as { __rowLog?: string[] };
        w.__rowLog = [];
        const cara = (n: Node): HTMLElement[] =>
          n instanceof HTMLElement
            ? [n, ...Array.from(n.querySelectorAll<HTMLElement>(".buddy-item"))].filter((e) => e.matches(".buddy-item") && (e.textContent ?? "").includes("Cara"))
            : [];
        new MutationObserver((records) => {
          for (const rec of records) {
            for (const n of Array.from(rec.addedNodes)) for (const f of cara(n)) w.__rowLog!.push(`${new Date().toISOString()} added in ${f.closest<HTMLElement>("[data-buddy-section]")?.dataset.buddySection ?? "none"}`);
            for (const n of Array.from(rec.removedNodes)) if (cara(n).length > 0) w.__rowLog!.push(`${new Date().toISOString()} removed`);
          }
        }).observe(document.body, { subtree: true, childList: true });
      }, row as unknown as HTMLElement);
      console.log(`[row] ${file}: before the click: ${await where()}`);
      await row.click();
      await peers[0].cli(["presence", "set", "--status", "online"]);
      await waitUntil(async () => (await sectionNow()) === "Online", NET_WAIT_MS, "Cara never came back online");
      await browser.pause(1000);
      const kept = (await browser.execute(() => {
        const r = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((e) => (e.textContent ?? "").includes("Cara"));
        return { same: (r as (HTMLElement & { __rowOnce?: number }) | undefined)?.__rowOnce === 1, signals: document.querySelector<HTMLElement>(".skin-tree")?.dataset.skinSignals ?? "no tree" };
      })) as { same: boolean; signals: string };
      console.log(`[row] ${file}: after selecting Cara: ${JSON.stringify(kept)}`);
      console.log(`[row] ${file}: after the push: ${await where()}; rows added/removed: ${JSON.stringify(await browser.execute(() => (window as unknown as { __rowLog?: string[] }).__rowLog ?? []))}`);
      await shot(`${file}-row-selected`);
      expect({ pack: file, rowKept: kept.same }).toEqual({ pack: file, rowKept: true });
      // WCAG 1.3.1 (lead): the row is described by its section's label,
      // which is a heading naming the section it now sits in.
      const named = (await browser.execute(() => {
        const r = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((e) => (e.textContent ?? "").includes("Cara"));
        const label = r?.getAttribute("aria-describedby") ? document.getElementById(r.getAttribute("aria-describedby")!) : null;
        return { role: label?.getAttribute("role") ?? "none", level: label?.getAttribute("aria-level") ?? "none", text: (label?.textContent ?? "").trim() };
      })) as { role: string; level: string; text: string };
      console.log(`[row] ${file}: Cara's section label: ${JSON.stringify(named)}`);
      expect({ pack: file, role: named.role, names: named.text.startsWith("Online") }).toEqual({ pack: file, role: "heading", names: true });
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
