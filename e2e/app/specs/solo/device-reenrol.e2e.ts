// Plan E3.2 S7d-keys.10 (frontend half), the path the lead set
// (2026-11-26): a copy of Ari's data root (a restored backup) starts on a
// peer and re-enrols the device: the copy retires the device key both
// copies share and endorses a fresh one, as one DeviceReEnrol op (ADR 0022
// rule 4). The app, the original, sees its own key retired through its
// own-record watch (.9): the Host's "Signing stopped" alert, then the Host's
// ReEnrolDevice review at the next unlock (TrustChanging, every word the
// Host's), and a held deviceReEnrol op shows the Host's label in catch-up.
//
// Also the held review's standing (part 2b): it survives a reload and shows
// in a window opened later, a cancel clears it everywhere and the Host
// raises it again at the next unlock, and the alert is shown once.
//
// Its contract
// (contract-s7dk10b-2026-11-26.md): `rekindle community re-enrol-device`
// (no community flag: one DeviceReEnrol in every community the data root
// holds; TrustChanging, so `--force` is refused), the alert "Signing
// stopped", the review IntentKind::ReEnrolDevice raised by the Host at the
// unlock barrier (subject "this device's key in every community"), and the
// catch-up label "Re-enrolling this device".

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, gone, resolveIntent, shown, switchTo, typeInto, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerReviewed, startClone, stopFleet } from "../../fleet.ts";
import { createCommunity, createIdentity, logOut, newUser, openCommunity, type User } from "../../journeys.ts";
import { showSkinView } from "../../journeys_skins.ts";
import { BUDDY, LOGIN, SETTINGS, inBuddy } from "../../../walk/nav.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = "Restore Hall";

/** The CLI subcommand that commits ReEnrolDevice on the copy (.10 part 2). */
const REENROL = ["community", "re-enrol-device"];

/** Log off and unlock again (the Host raises held reviews at the unlock barrier). */
async function unlockAgain(): Promise<void> {
  await logOut(browser, ME);
  await switchTo(browser, LOGIN, NET_WAIT_MS);
  await typeInto(await shown(browser, 'form.login-container input[type="password"]'), ME.passphrase);
  const submit = await browser.$('form.login-container button[type="submit"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Unlock stayed disabled (Connecting…)" });
  await submit.click();
}

/** What the review the window shows says, once it shows. */
async function reviewSays(b: typeof browser): Promise<string> {
  return ((await (await shown(b, ".intent-dialog", NET_WAIT_MS)).getText()) ?? "").replace(/\s+/g, " ");
}

const ME: User = { ...newUser("restore"), displayName: "Ari" };

describe("a restored copy re-enrols the device: the original is told and offered its own re-enrolment (S7d-keys.10)", function () {
  this.timeout(1_200_000);
  const me = ME;
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
    // list, in the Host's words.
    await unlockAgain();
    const said = await reviewSays(browser);
    console.log(`[reenrol] review: ${said.slice(0, 400)}`);
    await shot("reenrol-review");
    expect(said).toContain("this device's key in every community");
    expect(said).toContain("this device's key is retired at seq");

    // The review is the Host's, held (standing heldReviews on every subscribe
    // and resume): a reload of the window shows it again, and a window opened
    // now shows it on its first subscribe.
    await browser.refresh();
    expect(await reviewSays(browser)).toContain("this device's key in every community");
    await shot("reenrol-review-after-reload");
    await browser.execute(async () => {
      const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } }).__TAURI_INTERNALS__;
      await invoke("open_settings_window", { tab: null });
    });
    await switchTo(browser, SETTINGS, NET_WAIT_MS);
    expect(await reviewSays(browser)).toContain("this device's key in every community");
    await shot("reenrol-review-in-new-window");

    // Cancelled from the new window, it is gone from every window; the Host
    // raises it again at the next unlock, until it is committed.
    expect(await resolveIntent(browser, { passphrase: me.passphrase, cancel: true, appearMs: NET_WAIT_MS })).toBe("cancelled");
    await gone(browser, ".intent-dialog", NET_WAIT_MS);
    await switchTo(browser, BUDDY, NET_WAIT_MS);
    await gone(browser, ".intent-dialog", NET_WAIT_MS);
    await shot("reenrol-review-cancelled");
    await unlockAgain();
    expect(await reviewSays(browser)).toContain("this device's key in every community");

    // Committed: the dialog answers the review it shows (its holdId).
    const outcome = await resolveIntent(browser, { passphrase: me.passphrase, appearMs: NET_WAIT_MS });
    console.log(`[reenrol] review outcome: ${outcome}`);
    await shot("reenrol-review-done");
    expect(outcome).toBe("committed");

    // The Host's "Signing stopped" alert is one alert (de-duplicated by its
    // id) through the reload, the new window and two unlocks.
    await inBuddy(browser);
    await showSkinView(browser, "notifications");
    const alerts = (await browser.execute(() =>
      Array.from(document.querySelectorAll("[class*=notification]")).filter((n) => (n.textContent ?? "").includes("Signing stopped")).length,
    )) as number;
    console.log(`[reenrol] Signing stopped alerts shown: ${alerts}`);
    expect(alerts).toBe(1);

    // Its held op shows the Host's label in the community's catch-up status
    // until it lands, and is gone after.
    await waitUntil(
      async () => !(await browser.$("*=Re-enrolling this device").isExisting()),
      NET_WAIT_MS * 2,
      "the re-enrolment never landed (its catch-up label stayed)",
    );
  });
});
