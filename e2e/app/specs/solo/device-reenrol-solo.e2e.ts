// Plan E3.2 S7d-keys.9p2 (lead's ruling, 2026-11-27): a member alone in her
// community still learns that a copy of her device re-enrolled. Ari is the
// only member; her locked data root is copied to a second daemon, the copy
// falls behind its own record and re-enrols by CLI. The re-enrolment does not
// write the old record, and no other member carries it: the copy also writes
// a retirement pointer into the old device's own record (M(p,d)/2), so Ari's
// own-record watch fires and her Host shows "Signing stopped" with no
// carrier. Without the pointer the original keeps signing with a retired key,
// silently (the case the ruling refuses).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, resolveIntent, shown, waitUntil } from "../../../walk/helpers.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { type Peer, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { copyDivergesAndReEnrols, signingStoppedShown } from "../../reenrol.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = "Solo Hall";

describe("a solo member's original device learns its re-enrolment from its own record (S7d-keys.9p2)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("solo"), displayName: "Ari" };
  const peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari, alone, sees Signing stopped after her copy re-enrols", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    // Ari is the only member: nobody else can carry the retirement.
    await waitUntil(
      async () => (await browser.$$(".member-item[data-pseudonym]")).length === 1,
      NET_WAIT_MS,
      "Ari's member row did not appear in the solo community",
    );

    await copyDivergesAndReEnrols(browser, me, COMMUNITY, "Ari's copy", (copy) => peers.push(copy));

    // The Host opens the review as soon as the old record delivers the
    // retirement. Answer it before opening the notifications view.
    await inBuddy(browser);
    // Through the retirement pointer reaching Ari's own record (no carrier),
    // as long a bound as the alert had.
    const review = await shown(browser, ".intent-dialog", NET_WAIT_MS * 2);
    const reviewText = (await review.getText()).replace(/\s+/g, " ");
    expect(reviewText).toContain("this device's key in every community");
    expect(reviewText).toContain("this device's key is retired at seq");
    await shot("solo-reenrol-review");
    expect(
      await resolveIntent(browser, { passphrase: me.passphrase, cancel: true, appearMs: NET_WAIT_MS }),
    ).toBe("cancelled");
    await gone(browser, ".intent-dialog", NET_WAIT_MS);

    const alert = await signingStoppedShown(browser);
    console.log(`[reenrol] solo alert: ${alert.slice(0, 200)}`);
    await shot("solo-signing-stopped");
    expect(alert).toContain("Signing stopped");
  });
});
