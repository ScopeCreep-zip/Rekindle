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
import { artElements, frameNodeDetail, frameNodes, windowKind } from "../../skin-art.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const PACKS = ["combat", "alloy", "onyx", "xslimfire", "simple-x", "symbiosis", "total-chaos", "xfire-classic"];
/** The window whose frame a pack's community window wears (SKW1): its chat
 *  window's, or its buddy list's where the pack ships no chat window (Total
 *  Chaos, whose group chat has no pack frame either: a known finding). */
const FRAME_SOURCE: Record<string, "chat" | "buddy-list"> = { "total-chaos": "buddy-list" };
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
        if (w.label === "chat") {
          // Lead: a stray white bar at the 1:1 chat's top left (Combat): the
          // node drawn there and its box and art, to check against the pack.
          const at = (await browser.execute(() =>
            [8, 20].map((x) => {
              const el = document.elementFromPoint(x, 52) as HTMLElement | null;
              const node = el?.closest<HTMLElement>("[data-skin-node]");
              const r = node?.getBoundingClientRect();
              const cs = node ? getComputedStyle(node) : null;
              return `(${x},52) ${el?.tagName}.${el?.className} in ${node?.dataset.skinNode ?? "-"} (${node?.dataset.skinRole ?? "-"}) box ${r ? [r.x, r.y, r.width, r.height].map(Math.round) : "-"} bg ${cs?.backgroundColor ?? "-"} art ${(cs?.backgroundImage ?? "").slice(0, 80)}`;
            }),
          )) as string[];
          console.log(`[frame] ${file} chat top-left: ${JSON.stringify(at)}`);
        }
        // Named so a pack's windows sort side by side (chat, group chat, community).
        await shot(`${file}-${i}-${w.label}`);
        expect(kind).toBe(w.kind);
      }
      for (const w of windows) expect(counts[w.label]).toBeGreaterThan(0);
      // Frame parity (lead, SKW1): the group-chat and community windows wear
      // the pack's frame, the one its chat window draws (today the pack's
      // ChatWindow is every chat-like window's source): the same title strip,
      // window buttons and menus with the same art, and never a default title bar.
      const from = FRAME_SOURCE[file] ?? "chat";
      const source = frames[from];
      expect(source.nodes.length).toBeGreaterThan(0);
      expect(source.defaultTitlebar).toBe(false);
      // A pack with no chat window draws no chat frame for its group chat
      // (logged, not compared); its community window still wears its frame.
      const compared = from === "chat" ? ["group-chat", "community"] : ["community"];
      if (from !== "chat") console.log(`[frame] ${file}: no pack chat window; the community window is compared to the buddy list's frame, the group chat not`);
      // Evidence first (lead): for every node whose art differs from the
      // source's, what each window serves on it, the buddy list included
      // (is the art another window's, or another state's?).
      const byNode = (list: string[]): Map<string, string> => new Map(list.map((l) => [l.split(" ")[0], l]));
      const sourceBy = byNode(source.nodes);
      for (const label of compared) {
        const differing = [...byNode(frames[label].nodes)].filter(([id, line]) => sourceBy.get(id) !== line).map(([id]) => id);
        if (differing.length === 0) continue;
        for (const w of windows.filter((x) => ["buddy-list", from, label].includes(x.label))) {
          await browser.switchToWindow(w.handle);
          for (const line of await frameNodeDetail(differing)) console.log(`[frame-diff] ${file} ${label} vs ${from}, in ${w.label}: ${line}`);
        }
      }
      for (const label of compared) {
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
