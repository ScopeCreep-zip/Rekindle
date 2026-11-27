// Plan E3.4 S8-pre (contract-s8pre-2026-11-26.md, its real-app checks 1–3):
// a member's voice claim reaches the others through its beat, not the old
// registry poll. Ari (the app) creates a community and a voice channel;
// Cara (fleet) joins it by invite and joins the voice channel by CLI.
//   1. Ari then joins: Ari's voice panel lists Cara at once (Ari's join
//      hands the roster what Cara's beat shows), not after the 60 s poll.
//   2. Cara leaves: Ari's panel drops her within one beat delivery, not
//      after the 180 s registry TTL.
//   3. Cara joins again, then her daemon stops (a quit): her quit beat is
//      offline, so it claims no channel, and Ari's panel drops her.
// A fleet member with no community name is listed by fingerprint, so the
// panel is read by count (Ari plus Cara). Joining voice opens the
// microphone: Linux only (no TCC prompt may be raised on the Mac).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, gone, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { setValue } from "../../../walk/nav.ts";
import { joinFromStage, voiceNames } from "../../../walk/rows-voice.ts";
import { type Peer, peerJoinCommunity, startFleet, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, createInvite, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Beat Hall ${Date.now() % 100000}`;
const VOICE = "beat-call";
/** Beats are delivered within seconds; the poll this replaces waited 60 s and the registry TTL 180 s. */
const BEAT_MS = 45_000;

/** How many members Ari's voice panel lists. */
async function listed(): Promise<number> {
  const names = await voiceNames(browser);
  return names === "" ? 0 : names.split(" | ").length;
}

/** Wait until Ari's panel lists `count` members, within `ms`; how long it took. */
async function listsWithin(count: number, ms: number, what: string): Promise<number> {
  const from = Date.now();
  await waitUntil(async () => (await listed()) === count, ms, "").catch(async () => {
    throw new Error(`${what}: after ${ms} ms Ari's voice panel lists ${await listed()} (${await voiceNames(browser)}), not ${count}`);
  });
  const took = Date.now() - from;
  console.log(`[beats] ${what}: ${count} listed after ${took} ms (${await voiceNames(browser)})`);
  return took;
}

describe("voice join and leave reach the roster by beat (S8-pre)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("beats"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("a member already in voice is listed at once, and dropped on leave and on quit", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    // The voice channel, through the window's own Create Channel.
    await (await shown(browser, '[title="Create Channel"]')).click();
    await typeInto(await shown(browser, 'input[placeholder="Channel name..."]'), VOICE);
    await setValue(browser, ".modal-container select.form-select", "voice");
    await (await browser.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await gone(browser, 'input[placeholder="Channel name..."]', NET_WAIT_MS);

    peers = await startFleet(["Cara"]);
    const cara = peers[0];
    await peerJoinCommunity(cara, await createInvite(browser, me, 1));
    await openCommunity(browser, COMMUNITY);
    await cara.cli(["voice", "join", "--community", COMMUNITY, "--channel", VOICE]);

    // 1. Ari joins; Cara, already in, is listed at once.
    await (await byText(browser, ".channel-item", VOICE, NET_WAIT_MS)).click();
    await joinFromStage(browser);
    const joinTook = await listsWithin(2, BEAT_MS, "Ari joins with Cara already in");
    await shot("joined-cara-listed");

    // 2. Cara leaves; Ari's panel drops her within a beat.
    await cara.cli(["voice", "leave"]);
    const leaveTook = await listsWithin(1, BEAT_MS, "Cara leaves");
    await shot("cara-left");

    // 3. Cara joins again, then her daemon stops: her quit beat claims no channel.
    await cara.cli(["voice", "join", "--community", COMMUNITY, "--channel", VOICE]);
    await listsWithin(2, BEAT_MS, "Cara joins again");
    await cara.pause();
    const quitTook = await listsWithin(1, BEAT_MS, "Cara quits");
    await shot("cara-quit");

    expect({ joinTook: joinTook < BEAT_MS, leaveTook: leaveTook < BEAT_MS, quitTook: quitTook < BEAT_MS }).toEqual({
      joinTook: true,
      leaveTook: true,
      quitTook: true,
    });
  });
});
