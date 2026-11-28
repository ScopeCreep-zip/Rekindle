// Diagnostic (lead, group voice on Linux): what Symbiosis's GroupChatWindow
// tree draws in a group window. Ari (the app) and Bea and Cara (fleet) are a
// group; Ari applies Symbiosis and opens the group's window. Logged as
// `[gtree]`: whether the window lays a tree, and every node with words
// (id, role, action, label text, box), plus the shot.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, startFleet, stopFleet } from "../../fleet.ts";
import { createGroup, createIdentity, newUser, openGroup, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const GROUP = `Tree Squad ${Date.now() % 100000}`;

describe("Symbiosis's group window: what its tree draws", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("gtree"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("logs the group window's tree nodes with words", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea", "Cara"]);
    await befriendFleet(browser, me, peers);
    await createGroup(browser, me, GROUP, ["Bea", "Cara"]);
    await Promise.all(peers.map((p) => peerAcceptGroup(p, GROUP)));
    await openSettings(browser, "Appearance");
    const pack = builtinPacks().find((p) => p.file === "symbiosis");
    if (!pack) throw new Error("no built-in pack symbiosis");
    const href = await applySkin(browser, me, pack.name);
    await switchTo(browser, BUDDY);
    await openGroup(browser, GROUP);
    await skinShown(browser, href);
    await browser.pause(3000);
    const seen = (await browser.execute(() => {
      const tree = document.querySelector(".chat-window .skin-tree, .skin-tree");
      const out: string[] = [`tree: ${tree ? "yes" : "NO TREE"}; window class: ${document.querySelector(".app-frame, .chat-window")?.className ?? "?"}`];
      for (const n of Array.from(document.querySelectorAll<HTMLElement>("[data-skin-node]"))) {
        const label = (n.querySelector(":scope > .skin-node-label .skin-node-label-text")?.textContent ?? "").trim();
        const r = n.getBoundingClientRect();
        if (!label && !n.dataset.skinAction) continue;
        out.push(`${n.dataset.skinNode} role=${n.dataset.skinRole ?? "-"} action=${n.dataset.skinAction ?? "-"} label="${label}" box=${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)} hidden=${n.closest("[hidden]") !== null}`);
      }
      return out;
    })) as string[];
    for (const line of seen) console.log(`[gtree] ${line}`);
    await shot("group-window");
    expect(seen.length).toBeGreaterThan(0);
  });
});
