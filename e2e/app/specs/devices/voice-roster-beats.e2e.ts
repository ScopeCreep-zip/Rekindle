// Plan E3.4 S8-pre, its daemon half (contract-s8pre-2026-11-26.md): a member
// in voice through the CLI daemon reaches the others through its beat. Ari
// (the app) is in a voice channel; Cara (fleet) joins it by CLI.
//   - Within a few seconds of Cara's `voice join`, Ari's Host re-announces
//     its voice join to her (the presence reconcile, from her beat's claim),
//     read from Ari's own Host trace.
//   - After Cara's `voice leave`, her beat claims no channel, and Ari's Host
//     stops re-announcing.
// No voice-panel claim: the panel lists the voice transport's roster, which
// admits a peer once its voice engine answers, and the daemon has none
// before E4.1. Both members in real apps are voice-roster-peers. Joining
// voice opens the microphone: Linux only (no TCC prompt on the Mac).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, gone, resolveIntent, shown, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { setValue } from "../../../walk/nav.ts";
import { joinFromStage } from "../../../walk/rows-voice.ts";
import { type Peer, peerChannelId, peerJoinCommunity, peerVoiceJoin, peerVoiceLeave, startFleet, stopFleet } from "../../fleet.ts";
import { appLogPath, countInLog, RECONCILE_RESENT } from "../../host_log.ts";
import { createCommunity, createIdentity, createInvite, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Beat Hall ${Date.now() % 100000}`;
const VOICE = "beat-call";
/** A beat reaches Ari within seconds; Linux saw the first re-announce 1.5 s after the join. */
const ANNOUNCE_MS = 15_000;
/** Long enough for Cara's leave beat to arrive, then a quiet window in which no re-announce may follow. */
const SETTLE_MS = 15_000;
const QUIET_MS = 20_000;

describe("a CLI member's voice claim reaches the Host by beat (S8-pre, daemon half)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("beats"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari's Host re-announces to Cara after her CLI join, and stops after her leave", async () => {
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
    const cara = peers[0]!;
    await peerJoinCommunity(cara, await createInvite(browser, me, 1));
    await openCommunity(browser, COMMUNITY);
    // Ari is in voice first, so her Host reconciles its roster against beats.
    await (await byText(browser, ".channel-item", VOICE, NET_WAIT_MS)).click();
    await joinFromStage(browser);
    const log = appLogPath();

    // Cara joins by CLI (the voice commands take the channel's id).
    const voiceId = await peerChannelId(cara, COMMUNITY, VOICE);
    const before = countInLog(log, RECONCILE_RESENT);
    const joinedAt = Date.now();
    await peerVoiceJoin(cara, COMMUNITY, voiceId);
    await waitUntil(async () => countInLog(log, RECONCILE_RESENT) > before, ANNOUNCE_MS, "").catch(() => {
      throw new Error(`Ari's Host did not re-announce to Cara within ${ANNOUNCE_MS / 1000} s of her CLI voice join (no "${RECONCILE_RESENT}" in ${log})`);
    });
    const announceTook = Date.now() - joinedAt;
    console.log(`[beats] Ari's Host re-announced ${announceTook} ms after Cara's CLI voice join`);
    await shot("cara-joined-by-cli");

    // Cara leaves; once her leave beat is in, no further re-announce.
    await peerVoiceLeave(cara);
    await browser.pause(SETTLE_MS);
    const settled = countInLog(log, RECONCILE_RESENT);
    await browser.pause(QUIET_MS);
    const after = countInLog(log, RECONCILE_RESENT);
    console.log(`[beats] re-announces: ${settled - before} while Cara was in voice and settling, ${after - settled} in the ${QUIET_MS / 1000} s after`);

    expect({ announced: announceTook < ANNOUNCE_MS, quietAfterLeave: after - settled }).toEqual({ announced: true, quietAfterLeave: 0 });
  });
});
