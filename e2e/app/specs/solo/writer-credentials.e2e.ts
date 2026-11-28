// SEC6's real-app proof: a community's DHT writer credentials live in
// protected storage and are carried as protected writers through the record
// adapters, so records stay writable across an identity reload and readable
// by another member. Ari (the app) creates a community and posts in
// #general; logs off and unlocks again, so the Host reopens the identity and
// reads its writer credentials back from protected storage (the WDIO harness
// keeps the app process; logging off ends the identity's session and unlock
// starts it fresh); posts again; both posts show in his window. Bea (a fleet
// peer, her own Host and Veilid node) joins by Ari's invite and reads both
// posts from the channel's history through her CLI.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerJoinCommunity, startFleet, stopFleet } from "../../fleet.ts";
import {
  createCommunity,
  createIdentity,
  createInvite,
  logOut,
  newUser,
  openCommunity,
  postMessage,
  selectChannel,
  unlock,
  type User,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";

const b = browser;
const PLACE = "Writer Hall";

describe("writer credentials survive an identity reload (SEC6)", function () {
  this.timeout(1_200_000);
  const ari: User = { ...newUser("sec6"), displayName: "Ari" };
  const stamp = Date.now();
  const FIRST = `before the reload ${stamp}`;
  const SECOND = `after the reload ${stamp}`;
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari posts, reloads his identity, posts again; Bea joins and reads both", async () => {
    peers = await startFleet(["Bea"]);
    const [bea] = peers;
    await createIdentity(b, ari);
    await createCommunity(b, ari, PLACE);
    await openCommunity(b, PLACE);
    await selectChannel(b, "general");
    await postMessage(b, ari, FIRST);

    // The identity reloads: its writer credentials come back from protected storage.
    await logOut(b, ari);
    await unlock(b, ari);
    await openCommunity(b, PLACE);
    await selectChannel(b, "general");
    await postMessage(b, ari, SECOND);
    await byText(b, ".community-main .chat-message", FIRST.slice(0, 40), NET_WAIT_MS);
    await shot("both-posts-after-reload");

    // Bea joins and reads both from the channel's history.
    const link = await createInvite(b, ari, 1);
    await peerJoinCommunity(bea, link);
    let bodies: string[] = [];
    await waitUntil(
      async () => {
        const rows = (await bea.cli(["channel", "history", "-c", PLACE, "-C", "general"])) as
          | { body: string }[]
          | null;
        bodies = (rows ?? []).map((r) => r.body);
        return bodies.includes(FIRST) && bodies.includes(SECOND);
      },
      NET_WAIT_MS * 3,
      ""
    ).catch(() => {
      throw new Error(`Bea's history never held both posts (it holds ${JSON.stringify(bodies)})`);
    });
    console.log(`[sec6] Bea reads ${bodies.length} posts, both of Ari's among them`);
    await shot("ari-after-bea-joined");
    expect({ first: bodies.includes(FIRST), second: bodies.includes(SECOND) }).toEqual({
      first: true,
      second: true,
    });
  });
});
