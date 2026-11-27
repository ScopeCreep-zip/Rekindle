// Plan E3.2 S7d-keys.10 (frontend half), the path the lead set
// (2026-11-26): a copy of Ari's data root (a restored backup) starts on a
// peer and re-enrols the device: the copy retires the device key both
// copies share and endorses a fresh one, as one DeviceReEnrol op (ADR 0022
// rule 4). The app, the original, sees its own key retired through its
// own-record watch (.9): the Host's "Signing stopped" alert, then the Host's
// ReEnrolDevice review at the next unlock (TrustChanging, every word the
// Host's), and a held deviceReEnrol op shows the Host's label in catch-up.
//
// HELD until .10 part 2 lands: its contract names the CLI subcommand and
// flags that commit ReEnrolDevice (REENROL below) and the review's wording.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, resolveIntent, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerReviewed, startClone, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { showSkinView } from "../../journeys_skins.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = "Restore Hall";

/** The CLI subcommand that commits ReEnrolDevice on the copy, for community
 *  `anchor`: named by .10 part 2's contract (set it from there). */
const REENROL = (anchor: string): string[] => ["community", "reenrol-device", "--community", anchor];

describe("a restored copy re-enrols the device: the original is told and offered its own re-enrolment (S7d-keys.10)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("restore"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("the copy's re-enrolment retires the shared key: Signing stopped, then the Host's re-enrol review", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    const dataRoot = (browser.requestedCapabilities as Record<string, unknown>)["rekindle:dataRoot"];
    if (typeof dataRoot !== "string") throw new Error("the launcher handed no rekindle:dataRoot capability");

    // The copy: Ari's own files on a second daemon, unlocked with her passphrase.
    const copy = await startClone("Ari's copy", { dataRoot, passphrase: me.passphrase });
    peers = [copy];
    const communities = (await copy.cli(["community", "list"])) as { anchor: string; name: string }[] | null;
    const anchor = (communities ?? []).find((c) => c.name === COMMUNITY)?.anchor;
    if (!anchor) throw new Error(`the copy holds no ${COMMUNITY}: ${JSON.stringify(communities).slice(0, 200)}`);
    await peerReviewed(copy, REENROL(anchor));

    // The original sees its own key retired: the Host's alert, as served.
    await inBuddy(browser);
    await showSkinView(browser, "notifications");
    const alert = await byText(browser, "[class*=notification]", "Signing stopped", NET_WAIT_MS * 2);
    console.log(`[reenrol] alert: ${(await alert.getText()).replace(/\s+/g, " ").slice(0, 200)}`);
    await shot("signing-stopped");

    // The Host raises its ReEnrolDevice review (TrustChanging) for the
    // original; it is committed through the intent dialog, word for word.
    const outcome = await resolveIntent(browser, { passphrase: me.passphrase, appearMs: NET_WAIT_MS });
    console.log(`[reenrol] review: ${outcome}`);
    await shot("reenrol-review-done");
    expect(outcome).toBe("committed");

    // Its held op shows the Host's label in the community's catch-up status
    // until it lands, and is gone after.
    await waitUntil(
      async () => !(await browser.$("*=Re-enrolling this device").isExisting()),
      NET_WAIT_MS * 2,
      "the re-enrolment never landed (its catch-up label stayed)",
    );
  });
});
