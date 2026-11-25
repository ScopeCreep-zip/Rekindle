// Plan SB1: the Host scopes each pack's art to the windows the pack defines
// (`:root[data-window]`), so Settings gets none. The other half of the
// proof: no skinned window goes bare. Ari (the app) befriends Cara and Dan (fleet),
// so a 1:1 chat (Cara) and a group of three can open, and creates a community. For
// every pack, the buddy list, the 1:1 chat, the group chat and the community
// window each name their kind and paint pack art (`[art]` counts), each a
// shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings, underTree } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import { createCommunity, createGroup, createIdentity, friendsView, newUser, openChat, openCommunity, openGroup, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { artElements, frameNodes, windowKind } from "../../skin-art.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const PACKS = ["combat", "alloy", "onyx", "xslimfire", "simple-x", "symbiosis"];
const GROUP = `Art Squad ${Date.now() % 100000}`;
const COMMUNITY = `Art Hall ${Date.now() % 100000}`;

describe("skinned windows keep their pack art (SB1)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("skinart"), displayName: "Ari" };
  let peers: Peer[] = [];
  /** Each skinned window's handle and the kind its root must name. */
  const windows: { label: string; kind: string; handle: string }[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Dan"]);
    await befriendFleet(browser, me, peers);
    windows.push({ label: "buddy-list", kind: "buddy-list", handle: await browser.getWindowHandle() });
    await openChat(browser, "Cara");
    windows.push({ label: "chat", kind: "chat", handle: await browser.getWindowHandle() });
    await switchTo(browser, BUDDY);
    // A group DM is 3–8 people (NewGroupModal MIN_PICKED = 2 friends besides us).
    await createGroup(browser, me, GROUP, ["Cara", "Dan"]);
    await openGroup(browser, GROUP);
    windows.push({ label: "group-chat", kind: "group-chat", handle: await browser.getWindowHandle() });
    await switchTo(browser, BUDDY);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    windows.push({ label: "community", kind: "community", handle: await browser.getWindowHandle() });
  });

  after(async () => {
    await stopFleet(peers);
  });

  for (const file of PACKS) {
    it(`${file}: the buddy list, chat, group chat and community windows paint pack art`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      const counts: Record<string, number> = {};
      const frames: Record<string, { nodes: string[]; defaultTitlebar: boolean }> = {};
      for (const [i, w] of windows.entries()) {
        await browser.switchToWindow(w.handle);
        await skinShown(browser, href);
        await browser.pause(1000);
        const kind = await windowKind();
        const art = await artElements();
        counts[w.label] = art.length;
        frames[w.label] = await frameNodes();
        console.log(`[art] ${file} ${w.label}: data-window=${kind} ${art.length} element(s) with pack art`);
        console.log(`[frame] ${file} ${w.label}: default titlebar=${frames[w.label].defaultTitlebar}; ${JSON.stringify(frames[w.label].nodes)}`);
        // Named so a pack's windows sort side by side (chat, group chat, community).
        await shot(`${file}-${i}-${w.label}`);
        expect(kind).toBe(w.kind);
      }
      for (const w of windows) expect(counts[w.label]).toBeGreaterThan(0);
      // Frame parity (lead, SKW1): the group-chat and community windows wear
      // the pack's frame, the one its chat window draws (today the pack's
      // ChatWindow is every chat-like window's source): the same title strip,
      // window buttons and menus with the same art, and never a default title bar.
      const source = frames["chat"];
      expect(source.nodes.length).toBeGreaterThan(0);
      expect(source.defaultTitlebar).toBe(false);
      for (const label of ["group-chat", "community"]) {
        expect({ window: label, defaultTitlebar: frames[label].defaultTitlebar }).toEqual({ window: label, defaultTitlebar: false });
        expect({ window: label, frame: frames[label].nodes }).toEqual({ window: label, frame: source.nodes });
      }

      // The window's portaled surfaces (mounted under <body>, outside the
      // frame) are in the pack's scope too: its menu opens with its items,
      // and a friend row's context menu opens.
      await switchTo(browser, BUDDY);
      await friendsView(browser);
      if (await underTree(browser)) await press(browser, await shown(browser, '[data-skin-action="menu:file"]'));
      else await press(browser, await byText(browser, ".menu-bar-item", "Rekindle"));
      await shown(browser, ".menu-dropdown", NET_WAIT_MS);
      await byText(browser, ".menu-dropdown-item", "Profile");
      await shot("pack-menu");
      await browser.keys(["Escape"]);
      await waitUntil(
        async () => (await browser.execute(() => {
          const row = Array.from(document.querySelectorAll<HTMLElement>(".buddy-item")).find((r) => (r.textContent ?? "").includes("Cara"));
          if (!row) return false;
          const r = row.getBoundingClientRect();
          row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.x + 20, clientY: r.y + r.height / 2, button: 2 }));
          return true;
        })) as boolean,
        NET_WAIT_MS,
        "no friend row for Cara to open a context menu on",
      );
      await shown(browser, ".context-menu", NET_WAIT_MS);
      const items = (await browser.execute(() => document.querySelectorAll(".context-menu [role='menuitem']").length)) as number;
      console.log(`[art] ${file} menus: pack menu opened with Profile; context menu with ${items} item(s)`);
      await shot("context-menu");
      expect(items).toBeGreaterThan(0);
      await browser.keys(["Escape"]);
    });
  }
});
