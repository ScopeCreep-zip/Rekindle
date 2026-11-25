// FX3 (no local key mint): a member missing a rotated community key waits
// for a key holder, and is told. Cara (fleet) creates a community; Ari (the
// app) and Bea (fleet) join. Ari logs out; Cara kicks Bea, which rotates the
// key, and only Cara holds the new generation. Cara goes offline; Ari
// unlocks and learns the new generation from the governance state, with no
// holder online: his community window says "Waiting for the community key
// from a member…" (Pending; shot). Cara comes back, answers his key request,
// and the notice goes (Current; shot). The Host path is netsim
// key_status_after_unlock's.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerJoinCommunity, peerKick, peerMemberCount, peerOwnPseudonym, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, logOut, newUser, openCommunity, selectChannel, unlock, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Key Watch ${Date.now() % 100000}`;

/** The open community window's key notice text, or "" when none is drawn. */
async function keyNotice(): Promise<string> {
  return (await browser.execute(() => {
    const n = document.querySelector<HTMLElement>(".key-pending-notice");
    return n && n.getBoundingClientRect().height > 0 ? (n.textContent ?? "").trim() : "";
  })) as string;
}

describe("a rotated key we lack: pending until a holder answers, then current", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("keystatus"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea"]);
    const [cara, bea] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    await peerJoinCommunity(bea, await peerInvite(cara, anchor, 1));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 3, NET_WAIT_MS * 2, "Cara never counted three members");
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("while Ari is away Cara kicks Bea (the key rotates) and goes offline", async () => {
    const [cara] = peers;
    await logOut(browser, me);
    // The CLI kicks by pseudonym (the 64 hex community info serves).
    await peerKick(cara, anchor, await peerOwnPseudonym(peers[1], anchor));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) === 2, NET_WAIT_MS * 2, "Cara never counted Bea gone");
    await cara.pause();
  });

  it("Ari unlocks with no key holder online: the window says it waits for the key", async () => {
    await unlock(browser, me);
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "general");
    let notice = "";
    await waitUntil(async () => (notice = await keyNotice()).length > 0, NET_WAIT_MS * 2, "").catch(() => {
      throw new Error("no key-pending notice after unlock with no holder online");
    });
    console.log(`[key] pending: "${notice}"`);
    if (!notice.startsWith("Waiting for the community key from a member")) throw new Error(`unexpected notice: ${notice}`);
    await shot("key-pending");
  });

  it("Cara comes back and answers: the key is current and the notice goes", async () => {
    await peers[0].resume();
    await waitUntil(async () => (await keyNotice()) === "", NET_WAIT_MS * 3, "the key never became current after Cara came back");
    console.log("[key] current");
    await shot("key-current");
  });
});
