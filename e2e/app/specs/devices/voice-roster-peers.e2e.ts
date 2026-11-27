// Plan E3.4 S8-pre (contract-s8pre-2026-11-26.md, its real-app checks 1–3)
// with both members in real apps: a member's voice claim reaches the others
// through its beat, and the voice panel lists a peer once its voice engine
// answers the roster's introduction (rekindle-voice roster_reconcile). A CLI
// peer has no voice engine before E4.1, so the peer here is a second app
// (wdio.devices-peers.conf.ts: alice is Ari, bob is Cara).
//   1. Cara is in voice; Ari joins: Ari's panel lists Cara within one beat.
//   2. Cara leaves from her own control: Ari's panel drops her.
//   3. Cara joins again, then quits her app (its menu's Quit): her quit beat
//      claims no channel, and Ari's panel drops her.
// Each step's time is logged against the 45 s bound (the poll this replaces
// waited 60 s, the registry TTL 180 s). Joining voice opens the microphone:
// Linux only (no TCC prompt may be raised on the Mac).

import { multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, gone, press, resolveIntent, shown, switchTo, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, menu, setValue } from "../../../walk/nav.ts";
import { joinFromStage, voiceNames } from "../../../walk/rows-voice.ts";
import { appLogPath, countInLog, RECONCILE_RESENT } from "../../host_log.ts";
import { shot } from "../../shots.ts";
import { createCommunity, createIdentity, createInvite, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";

const app = (name: "alice" | "bob"): Browser => multiRemoteBrowser.getInstance(name) as unknown as Browser;

const COMMUNITY = `Beat Hall ${Date.now() % 100000}`;
const VOICE = "beat-call";
/** Beats are delivered within seconds; the poll this replaces waited 60 s and the registry TTL 180 s. */
const BEAT_MS = 45_000;

/** How many members `b`'s voice panel lists. */
async function listed(b: Browser): Promise<number> {
  const names = await voiceNames(b);
  return names === "" ? 0 : names.split(" | ").length;
}

/** Wait until `b`'s panel lists `count` members, within `ms`; how long it took. */
async function listsWithin(b: Browser, count: number, ms: number, what: string): Promise<number> {
  const from = Date.now();
  await waitUntil(async () => (await listed(b)) === count, ms, "").catch(async () => {
    throw new Error(`${what}: after ${ms} ms Ari's voice panel lists ${await listed(b)} (${await voiceNames(b)}), not ${count}`);
  });
  const took = Date.now() - from;
  console.log(`[beats] ${what}: ${count} listed after ${took} ms (${await voiceNames(b)})`);
  return took;
}

/** In `b`'s community window, open the voice channel and join it from its stage. */
async function joinVoice(b: Browser): Promise<void> {
  await (await byText(b, ".channel-item", VOICE, NET_WAIT_MS)).click();
  await joinFromStage(b);
}

/** `b` leaves voice from its own control bar. */
async function leaveVoice(b: Browser): Promise<void> {
  await press(b, await shown(b, '.call-stage [aria-label="Leave voice channel"]', NET_WAIT_MS));
  await gone(b, ".voice-panel", NET_WAIT_MS);
}

/** `b` quits its app from the buddy list's Rekindle menu (the tray's Quit):
 *  the pack's own menu under a skin, else the app's menu bar. */
async function quitApp(b: Browser): Promise<void> {
  await switchTo(b, BUDDY);
  await menu(b, "Rekindle", "Quit");
}

describe("voice join, leave and quit reach the roster by beat, both members in real apps (S8-pre)", function () {
  this.timeout(1_800_000);
  const ari: User = { ...newUser("beats-ari"), displayName: "Ari" };
  const cara: User = { ...newUser("beats-cara"), displayName: "Cara" };

  it("a member already in voice is listed at once, and dropped on leave and on quit", async () => {
    const a = app("alice");
    const c = app("bob");
    await createIdentity(a, ari);
    await createIdentity(c, cara);
    await createCommunity(a, ari, COMMUNITY);
    await openCommunity(a, COMMUNITY);
    // The voice channel, through the window's own Create Channel.
    await (await shown(a, '[title="Create Channel"]')).click();
    await typeInto(await shown(a, 'input[placeholder="Channel name..."]'), VOICE);
    await setValue(a, ".modal-container select.form-select", "voice");
    await (await a.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(a, { passphrase: ari.passphrase, appearMs: 3000 });
    await gone(a, 'input[placeholder="Channel name..."]', NET_WAIT_MS);

    await joinCommunity(c, cara, await createInvite(a, ari, 1), COMMUNITY);
    await openCommunity(c, COMMUNITY);
    await openCommunity(a, COMMUNITY);
    await joinVoice(c);

    // 1. Ari joins; Cara, already in, is listed within a beat. Ari's own Host
    // trace says whether its presence reconcile had to re-send the join.
    const ariLog = appLogPath("alice");
    const resentBefore = countInLog(ariLog, RECONCILE_RESENT);
    await joinVoice(a);
    const joinTook = await listsWithin(a, 2, BEAT_MS, "Ari joins with Cara already in");
    // Cara is named as the member list names her (the Host's shown name,
    // contract VN1): never a raw 64-hex key, in the voice panel or on the stage.
    const rawKeys = (await a.execute(() =>
      Array.from(document.querySelectorAll(".voice-panel, .call-stage"))
        .map((el) => el.textContent ?? "")
        .join(" ")
        .match(/[0-9a-f]{64}/gi) ?? [],
    )) as string[];
    console.log(`[beats] raw keys in Ari's voice panel and stage: ${rawKeys.length}`);
    await shot("named-in-voice");
    if (rawKeys.length > 0) throw new Error(`Ari's voice panel or stage names a member by a raw key (${rawKeys.length} found)`);
    console.log(`[beats] Ari's presence reconcile re-sent her voice join ${countInLog(ariLog, RECONCILE_RESENT) - resentBefore} time(s) before Cara was listed`);

    // 2. Cara leaves from her own control; Ari's panel drops her.
    await leaveVoice(c);
    const leaveTook = await listsWithin(a, 1, BEAT_MS, "Cara leaves");

    // 3. Cara joins again, then quits her app: her quit beat claims no channel.
    await joinVoice(c);
    await listsWithin(a, 2, BEAT_MS, "Cara joins again");
    await quitApp(c);
    const quitTook = await listsWithin(a, 1, BEAT_MS, "Cara quits her app");

    console.log(`[beats] join ${joinTook} ms, leave ${leaveTook} ms, quit ${quitTook} ms (bound ${BEAT_MS} ms)`);
  });
});
