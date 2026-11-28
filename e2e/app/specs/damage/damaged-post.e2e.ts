// KP: a channel post whose key we hold but whose body does not open is
// stored and shown, in the Host's words, never skipped or shown as waiting.
// Ari (the app, launched with the Host's damage switch armed:
// wdio.damage.conf.ts) creates a community; Bea (fleet) joins and posts in
// general. The first post Ari's Host receives has one ciphertext byte
// flipped, so its real AEAD fails: Ari's open channel shows the damaged
// bubble live (shot), and again after the community window is reopened
// and its history read (shot). A second post from Bea opens normally.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerJoinCommunity, peerPostChannel, startFleet, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, createInvite, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Damage Hall ${Date.now() % 100000}`;
const CHANNEL = "general";
/** The Host's words for a post that does not open under its key (KP). */
const DAMAGED = "This message could not be read";

/** The damaged bubbles in the open channel, by text. */
async function damagedShown(): Promise<string[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll(".community-main .message-damaged")).map((n) => (n.textContent ?? "").trim()),
  )) as string[];
}

describe("a damaged channel post shows in the Host's words, live and from history", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("damage"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Bea"]);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, CHANNEL);
    await peerJoinCommunity(peers[0], await createInvite(browser, me, 1));
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("Bea's first post arrives damaged: the bubble says so, never 'waiting'", async () => {
    await peerPostChannel(peers[0], COMMUNITY, CHANNEL, `first ${Date.now()}`);
    let shown: string[] = [];
    await waitUntil(async () => (shown = await damagedShown()).length > 0, NET_WAIT_MS * 2, "").catch(() => {
      throw new Error(`no damaged bubble arrived (shown: ${JSON.stringify(shown)})`);
    });
    expect(shown[0]).toContain(DAMAGED);
    expect(shown[0]).not.toContain("waiting");
    await shot("damaged-live");
  });

  it("Bea's next post opens normally", async () => {
    const text = `second ${Date.now()}`;
    await peerPostChannel(peers[0], COMMUNITY, CHANNEL, text);
    await waitUntil(
      async () =>
        (await browser.execute(
          (t: string) => Array.from(document.querySelectorAll(".community-main .chat-message")).some((m) => (m.textContent ?? "").includes(t)),
          text,
        )) as boolean,
      NET_WAIT_MS * 2,
      "Bea's second post never showed",
    );
    expect(await damagedShown()).toHaveLength(1);
  });

  it("after reopening the community, its history still shows the damaged post", async () => {
    await browser.closeWindow();
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, CHANNEL);
    await waitUntil(async () => (await damagedShown()).length === 1, NET_WAIT_MS, "the damaged post is not in the reopened history");
    expect((await damagedShown())[0]).toContain(DAMAGED);
    await shot("damaged-history");
  });
});
