// Diagnostic (lead, perms9 shot "MEMBERS — 0"): what the Host answers for a
// joined community's members, beside what the window draws. Cara (fleet)
// creates a community and Ari (the app) joins; with Ari's community window
// open, `[members]` logs the Host's get_community_members answer (called in
// the page) and the drawn "Members — N" header. Run on trunk and on CAPR.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerMemberCount, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Count Hall ${Date.now() % 100000}`;

describe("a joined community's members: the Host's answer and the drawn count", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("mcount"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("logs get_community_members and the header", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    const anchor = await peerCreateCommunity(peers[0], COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(peers[0], anchor, 1), COMMUNITY);
    await waitUntil(async () => (await peerMemberCount(peers[0], anchor)) >= 2, NET_WAIT_MS * 2, "Cara never counted Ari");
    await openCommunity(browser, COMMUNITY);
    await browser.pause(15000);
    const seen = (await browser.execute(async (name: string) => {
      const header = (document.querySelector(".member-list-header")?.textContent ?? "no header").trim();
      const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: (cmd: string, args?: unknown) => Promise<unknown> } };
      let answer = "";
      try {
        const list = (await w.__TAURI_INTERNALS__!.invoke("get_communities")) as unknown;
        // A Host answer comes as { value, undo } (hostInvoke's Done).
        const rows = ((list as { value?: unknown }).value ?? []) as { anchor?: string; id?: string; name?: string }[];
        const id = rows.find((r) => r.name === name)?.anchor ?? rows.find((r) => r.name === name)?.id ?? "";
        const raw = await w.__TAURI_INTERNALS__!.invoke("get_community_members", { communityId: id });
        const members = ((raw as { value?: unknown }).value ?? raw) as unknown[];
        answer = `community ${id.slice(0, 16)}… members ${Array.isArray(members) ? members.length : "not a list"}: ${JSON.stringify(raw).slice(0, 600)}`;
      } catch (e) {
        answer = `error: ${String(e).slice(0, 300)}`;
      }
      return { header, answer };
    }, COMMUNITY)) as { header: string; answer: string };
    console.log(`[members] header: ${seen.header}`);
    console.log(`[members] host: ${seen.answer}`);
    await shot("members");
    expect(seen.header.length).toBeGreaterThan(0);
  });
});
