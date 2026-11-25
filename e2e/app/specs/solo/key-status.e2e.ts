// FX3 (no local key mint): a member missing a rotated community key waits
// for a key holder, and is told. Cara (fleet) creates a community; Ari (the
// app), Bea, Dan and Eve (fleet) join: five, so a rotation can be confirmed
// without Ari (ADR 0018: a strict majority of the previous epoch's eligible
// rotators confirms it; with three, Ari's own confirmation would be needed).
// Ari logs out; Cara kicks Bea, which starts the rotation; Cara, Dan and Eve
// confirm generation 2 (waited on Cara's `community info`). The three go
// offline; Ari unlocks and learns generation 2 from the governance state,
// with no holder online: his community window says "Waiting for the
// community key from a member…" (Pending; shot). Cara comes back, answers his
// key request, and the notice goes (Current; shot). The Host path is netsim
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
    peers = await startFleet(["Cara", "Bea", "Dan", "Eve"]);
    const [cara, ...others] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    for (const peer of others) await peerJoinCommunity(peer, await peerInvite(cara, anchor, 1));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 5, NET_WAIT_MS * 3, "Cara never counted five members");
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("while Ari is away Cara kicks Bea, the rotation is confirmed without Ari, and every key holder goes offline", async () => {
    const [cara, bea, dan, eve] = peers;
    await logOut(browser, me);
    // The CLI kicks by pseudonym (the 64 hex community info serves).
    await peerKick(cara, anchor, await peerOwnPseudonym(bea, anchor));
    // The key holders go offline only once generation 2 is confirmed (Cara's
    // `community info` `mekGeneration`, the confirmed one) and her kick and
    // the rotation have landed on the DHT: no op of hers still publishing
    // (`opPublications` empty) and no barrier holding her writes (`catchUp`
    // absent). Her member count flips locally before publication.
    let last = "";
    await waitUntil(
      async () => {
        const info = ((await cara.cli(["community", "info", "--community", anchor])) ?? {}) as Record<string, unknown>;
        const members = (info.memberCount ?? info.member_count) as number | undefined;
        const publishing = (info.opPublications ?? info.op_publications ?? {}) as Record<string, unknown>;
        const catchUp = info.catchUp ?? info.catch_up;
        const generation = (info.mekGeneration ?? info.mek_generation) as number | undefined;
        last = JSON.stringify({ members, generation, publishing, catchUp });
        return members === 4 && generation === 2 && Object.keys(publishing).length === 0 && catchUp == null;
      },
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(`generation 2 was never confirmed and landed (Cara's community info: ${last})`);
    });
    console.log(`[key] generation 2 confirmed, Cara's ops landed: ${last}`);
    // Every member that holds generation 2 goes offline: no holder can answer Ari.
    await Promise.all([cara.pause(), dan.pause(), eve.pause()]);
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
