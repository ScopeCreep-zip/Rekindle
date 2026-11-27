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
// raises it again at the next unlock, and the alert is shown once. With a
// second held review (Dee, a CLI friend, resets her session: the Host holds
// AcceptSessionReset), the answer to the review on show resolves that one
// only (its holdId), and the other shows next.
//
// Its contract
// (contract-s7dk10b-2026-11-26.md): `rekindle community re-enrol-device`
// (no community flag: one DeviceReEnrol in every community the data root
// holds; TrustChanging, so `--force` is refused), the alert "Signing
// stopped", the review IntentKind::ReEnrolDevice raised by the Host at the
// unlock barrier (subject "this device's key in every community"), and the
// catch-up label "Re-enrolling this device".

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerJoinCommunity, peerRequestFriend, startFleet, stopFleet } from "../../fleet.ts";
import { acceptRequest, createCommunity, createIdentity, createInvite, friendCode, logOut, newUser, openCommunity, pendingRequest, type User } from "../../journeys.ts";
import { showSkinView } from "../../journeys_skins.ts";
import { BUDDY, SETTINGS, inBuddy } from "../../../walk/nav.ts";
import { shot } from "../../shots.ts";
import { pageLog, recordPage } from "../../../walk/probe.ts";
import { copyDivergesAndReEnrols, signingStoppedShown, unlock } from "../../reenrol.ts";

const COMMUNITY = "Restore Hall";

/** Log off and unlock again (the Host raises held reviews at the unlock barrier). */
async function unlockAgain(): Promise<void> {
  await logOut(browser, ME);
  await unlock(browser, ME);
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
    // Dee, a friend over the CLI: later her session reset is a second
    // review the Host holds, beside the re-enrolment.
    const fleet = await startFleet(["Dee"]);
    peers = [...fleet];
    const dee = fleet[0]!;
    const hello = "hello from Dee";
    await peerRequestFriend(dee, await friendCode(browser), hello);
    await pendingRequest(browser, hello);
    await acceptRequest(browser, me, hello);
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    // Dee is a member too: a re-enrolment does not write the old record, so
    // the original learns of it from another member who merged the op and
    // carries its locator (the netsim's cloned-root case has four nodes).
    // She is admitted (her row shows) before the copy is taken.
    await peerJoinCommunity(dee, await createInvite(browser, me, 1));
    await waitUntil(
      async () => (await browser.$$(".member-item[data-pseudonym]")).length >= 2,
      NET_WAIT_MS * 2,
      "Dee's member row never showed in Ari's community",
    );
    // The copy, left behind its own record by an op of Ari's, re-enrols
    // (reenrol.ts: the locked root copied, as the netsim's cloned-root case).
    await copyDivergesAndReEnrols(browser, me, COMMUNITY, "Ari's copy", (copy) => peers.push(copy));

    // The original sees its own key retired: the Host's alert, as served.
    console.log(`[reenrol] alert: ${(await signingStoppedShown(browser)).slice(0, 200)}`);
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

    // A second held review: Dee resets her session with Ari, so Ari's Host
    // also holds AcceptSessionReset. The dialog shows the first held (the
    // re-enrolment); its answer must resolve that review only (its holdId),
    // and the session reset must then show.
    const deeFriends = (await dee.cli(["friend", "list"])) as { publicKey?: string; displayName?: string }[] | null;
    const ariKey = (deeFriends ?? []).find((f) => f.displayName === me.displayName)?.publicKey ?? (deeFriends ?? [])[0]?.publicKey;
    if (!ariKey) throw new Error(`Dee's friend list has no Ari: ${JSON.stringify(deeFriends).slice(0, 200)}`);
    // The second review is gated on the Host's own event: the window's
    // channel delivers an IntentPending whose review is acceptSessionReset
    // (walk/probe.ts wraps the channel's callback), while the re-enrolment
    // is still the review on show. Only then is the first one answered.
    await recordPage(browser, [], ['"kind":"acceptSessionReset"']);
    await dee.cli(["friend", "reset-session", ariKey]);
    await waitUntil(
      async () => /\+\d+ms event /.test(await pageLog(browser)),
      NET_WAIT_MS,
      "the Host did not hold Dee's session reset while re-enrolment was still open",
    );
    // The probe really saw the channel (a wrapper on invoke once saw nothing):
    // every delivery it logged before the gate, by kind.
    const seen = (await pageLog(browser)).split("\n").filter((l) => /^\+\d+ms /.test(l));
    console.log(`[reenrol] probe saw ${seen.length} channel deliveries before the gate, ${seen.filter((l) => / event /.test(l)).length} of them acceptSessionReset`);
    expect(seen.filter((l) => / event /.test(l)).length).toBeGreaterThan(0);
    expect(await reviewSays(browser)).toContain("this device's key in every community");
    await shot("two-held-reviews");

    // Committed: the dialog answers the review it shows (its holdId).
    const outcome = await resolveIntent(browser, { passphrase: me.passphrase, appearMs: NET_WAIT_MS });
    console.log(`[reenrol] review outcome: ${outcome}`);
    expect(outcome).toBe("committed");
    // Only the re-enrolment resolved: the session reset is now the review on show.
    let next = "";
    await waitUntil(
      async () => {
        next = ((await (await browser.$(".intent-dialog")).getText().catch(() => "")) ?? "").replace(/\s+/g, " ");
        // The probe above saw the Host hold the session reset (its
        // acceptSessionReset review) while the re-enrolment was on show, so
        // the next review shown is that one: anything but the re-enrolment.
        return next !== "" && !next.includes("this device's key in every community");
      },
      NET_WAIT_MS,
      "after the re-enrolment was committed the session reset review never showed",
    );
    console.log(`[reenrol] next held review: ${next.slice(0, 300)}`);
    await shot("session-reset-review-after-commit");
    // Cancelled, it leaves; the committed re-enrolment does not come back.
    expect(await resolveIntent(browser, { passphrase: me.passphrase, cancel: true, appearMs: NET_WAIT_MS })).toBe("cancelled");
    await gone(browser, ".intent-dialog", NET_WAIT_MS);
    await shot("reenrol-review-done");

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
