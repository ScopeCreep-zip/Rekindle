// Plan E3.2 S7d-keys.10 (frontend half), the path the lead set
// (2026-11-26): a copy of Ari's data root (a restored backup) starts on a
// peer and re-enrols the device: the copy retires the device key both
// copies share and endorses a fresh one, as one DeviceReEnrol op (ADR 0022
// rule 4). The app, the original, sees its own key retired through its
// own-record watch (.9): the Host's "Signing stopped" alert, then the Host's
// ReEnrolDevice review at the next unlock (TrustChanging, every word the
// Host's), and a held deviceReEnrol op shows the Host's label in catch-up.
//
// HELD until .10 part 2 lands. Its contract
// (contract-s7dk10b-2026-11-26.md): `rekindle community re-enrol-device`
// (no community flag: one DeviceReEnrol in every community the data root
// holds; TrustChanging, so `--force` is refused), the alert "Signing
// stopped", the review IntentKind::ReEnrolDevice raised by the Host at the
// unlock barrier (subject "this device's key in every community"), and the
// catch-up label "Re-enrolling this device".

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, resolveIntent, shown, switchTo, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerReviewed, startClone, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, logOut, newUser, openCommunity, type User } from "../../journeys.ts";
import { showSkinView } from "../../journeys_skins.ts";
import { LOGIN, inBuddy } from "../../../walk/nav.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = "Restore Hall";

/** The CLI subcommand that commits ReEnrolDevice on the copy (.10 part 2). */
const REENROL = ["community", "re-enrol-device"];

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

    // The copy: Ari's own files on a second daemon, unlocked with her passphrase;
    // it must hold the community before it re-enrols in it.
    const copy = await startClone("Ari's copy", { dataRoot, passphrase: me.passphrase });
    peers = [copy];
    const communities = (await copy.cli(["community", "list"])) as { name?: string }[] | null;
    if (!(communities ?? []).some((c) => c.name === COMMUNITY)) throw new Error(`the copy holds no ${COMMUNITY}: ${JSON.stringify(communities).slice(0, 200)}`);
    await peerReviewed(copy, REENROL);

    // The original sees its own key retired: the Host's alert, as served.
    await inBuddy(browser);
    await showSkinView(browser, "notifications");
    const alert = await byText(browser, "[class*=notification]", "Signing stopped", NET_WAIT_MS * 2);
    console.log(`[reenrol] alert: ${(await alert.getText()).replace(/\s+/g, " ").slice(0, 200)}`);
    await shot("signing-stopped");

    // The Host raises its ReEnrolDevice review (TrustChanging) at the unlock
    // barrier: log off, unlock, and the review is there before the buddy
    // list, in the Host's words; it is committed through the intent dialog.
    await logOut(browser, me);
    await switchTo(browser, LOGIN, NET_WAIT_MS);
    await typeInto(await shown(browser, 'form.login-container input[type="password"]'), me.passphrase);
    const submit = await browser.$('form.login-container button[type="submit"]');
    await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Unlock stayed disabled (Connecting…)" });
    await submit.click();
    const review = await shown(browser, ".intent-dialog", NET_WAIT_MS);
    const said = (await review.getText()).replace(/\s+/g, " ");
    console.log(`[reenrol] review: ${said.slice(0, 400)}`);
    await shot("reenrol-review");
    expect(said).toContain("this device's key in every community");
    expect(said).toContain("this device's key is retired at seq");
    const outcome = await resolveIntent(browser, { passphrase: me.passphrase, appearMs: NET_WAIT_MS });
    console.log(`[reenrol] review outcome: ${outcome}`);
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
