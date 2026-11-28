// Contract VN2 (call and video names): a community call's video tile is
// labelled with its sender's name as the Host serves it, the voice roster's
// shown name, never empty and never a key: the community call stage's
// tile (call_stage/ParticipantTile.tsx) names it over the video.
// Under ADR 0016 a frame decodes only with the sender-owned key the Host
// hands out on roster admission, so a drawn tile's sender is already in the
// served roster; an empty label would be a UI ordering gap. Every sample
// counts, from the first remote tile drawn.
//
// Two real apps (wdio.devices-peers.conf.ts: alice, bob). Alice makes a
// community and its voice channel; Bob joins by invite. Each names their
// community card (the roster's shown name, VN1). Both join the voice channel
// from the call stage; Bob starts his camera from the call controls. Alice's
// remote tile must read "Bob" at every sample. A voice channel opens the
// microphone and a camera opens video capture: Linux only (no TCC prompt on
// the Mac).

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import {
  byText,
  gone,
  NET_WAIT_MS,
  resolveIntent,
  shown,
  typeInto,
  WAIT_MS,
  waitUntil,
} from "../../../walk/helpers.ts";
import { setValue } from "../../../walk/nav.ts";
import {
  createCommunity,
  createIdentity,
  createInvite,
  joinCommunity,
  newUser,
  openCommunity,
  setCommunityName,
  type User,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";

const app = (name: "alice" | "bob"): Browser =>
  multiRemoteBrowser.getInstance(name) as unknown as Browser;

const PLACE = "Video Hall";
const VOICE = "Lounge";
/** `b` joins the voice channel from the call stage's Join button (CallStage.tsx). */
async function joinVoice(b: Browser): Promise<void> {
  await (await byText(b, ".channel-item", VOICE, NET_WAIT_MS)).click();
  await (await shown(b, ".call-stage-join-btn", WAIT_MS)).click();
  await shown(b, ".voice-panel", NET_WAIT_MS);
}

/** The names on the remote video tiles of `b`'s call stage (ParticipantTile:
 *  a stage tile with a decoder canvas that is not mirrored; only our own
 *  camera mirrors). Each tile's name is counted, empty or not. */
async function remoteLabels(b: Browser): Promise<string[]> {
  return (await b.execute(() =>
    Array.from(document.querySelectorAll(".call-stage-tile"))
      .filter(
        (t) =>
          t.querySelector(".call-stage-tile-canvas") !== null &&
          !t.classList.contains("call-stage-tile-mirror")
      )
      .map((t) =>
        (t.querySelector(".call-stage-tile-name")?.textContent ?? "").replace(/\s+/g, " ").trim()
      )
  )) as string[];
}

describe("a community call's video tiles carry the Host's names (VN2)", function () {
  this.timeout(1_800_000);
  const alice: User = { ...newUser("vt-alice"), displayName: "Alice" };
  const bob: User = { ...newUser("vt-bob"), displayName: "Bob" };

  it("Alice's tile for Bob's camera reads Bob from its first frame on", async () => {
    if (process.platform === "darwin")
      throw new Error(
        "macOS: a voice channel and a camera open capture (a TCC check); run this on Linux"
      );
    const a = app("alice");
    const b = app("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);

    // Alice's community, its voice channel (as show-voice makes one), and an invite.
    await createCommunity(a, alice, PLACE);
    await openCommunity(a, PLACE);
    await setCommunityName(a, alice, alice.displayName);
    await (await shown(a, '[title="Create Channel"]')).click();
    await typeInto(await shown(a, 'input[placeholder="Channel name..."]'), VOICE);
    await setValue(a, ".modal-container select.form-select", "voice");
    await (await a.$('.modal-container button[type="submit"]')).click();
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 3000 });
    await gone(a, 'input[placeholder="Channel name..."]', NET_WAIT_MS);
    const link = await createInvite(a, alice, 1);

    // Bob joins and names his card.
    await joinCommunity(b, bob, link, PLACE);
    await openCommunity(b, PLACE);
    await setCommunityName(b, bob, bob.displayName);

    // Both in the voice channel; Alice's roster shows Bob by his served name.
    await joinVoice(a);
    await joinVoice(b);
    await byText(a, ".voice-panel", bob.displayName, NET_WAIT_MS);

    // Bob starts his camera from the call controls (CallControlBar.tsx).
    await (await shown(b, '[aria-label^="Start camera"]', NET_WAIT_MS)).click();
    await shown(b, '[aria-label="Stop camera"]', WAIT_MS);

    // Alice: every sample from her first remote tile on, for 10 s more.
    const off: string[][] = [];
    let samples = 0;
    let first: string[] = [];
    await waitUntil(
      async () => {
        first = await remoteLabels(a);
        return first.length > 0;
      },
      NET_WAIT_MS,
      ""
    ).catch(() => {
      throw new Error("Alice never drew a remote video tile for Bob's camera");
    });
    for (let labels = first, until = Date.now() + 10_000; ; labels = await remoteLabels(a)) {
      samples += 1;
      if (labels.some((l) => l !== bob.displayName) && off.length < 10) off.push(labels);
      if (Date.now() >= until) break;
      await new Promise((r) => setTimeout(r, 250));
    }
    const last = await remoteLabels(a);
    const hexRuns =
      last
        .join(" ")
        .toLowerCase()
        .match(/[0-9a-f]{8,}/g) ?? [];
    console.log(
      `[video-tile] first remote labels ${JSON.stringify(first)}; ${samples} samples; off samples (a label empty or not the served name): ${JSON.stringify(off)}; last ${JSON.stringify(last)}`
    );
    await shot("video-tile-names");

    expect({ firstSample: first, off, last, hexRuns }).toEqual({
      firstSample: [bob.displayName],
      off: [],
      last: [bob.displayName],
      hexRuns: [],
    });
  });
});
