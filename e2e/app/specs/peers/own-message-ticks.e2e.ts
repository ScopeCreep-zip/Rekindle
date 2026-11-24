// Plan E6.11 (F-echo's owed half): our own message's delivery state, as the
// Host serves it. Its row arrives `queued` (stored, not yet sent: the pending
// clock), the outbox reports `sent` (one tick) and the peer's receipt
// `delivered` (two ticks). Two apps (multiremote, wdio.peers.conf.ts): Alice
// and Bob are friends; Bob logs out, so no receipt can come; Alice's message
// reaches `sent` and stays there while he is away, never back on the clock
// (the send's commit-time answer used to replace the row and put a sent
// message back on the clock: Linux, 2026-11-23); Bob unlocks, and it becomes
// `delivered`. Each state is a shot.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, resolveIntent, sleep, typeAndSend, waitUntil } from "../../../walk/helpers.ts";
import { acceptRequest, createIdentity, friendCode, friendRow, logOut, newUser, openChat, pendingRequest, requestFriend, unlock } from "../../journeys.ts";
import { shot } from "../../shots.ts";

/** The delivery state `b`'s open chat shows on its one own row for `text` ("" while absent). */
async function ownState(b: Browser, text: string): Promise<string> {
  return (await b.execute((t: string) => {
    const rows = Array.from(document.querySelectorAll(".chat-message")).filter((m) => (m.textContent ?? "").includes(t));
    if (rows.length !== 1) return rows.length === 0 ? "" : `${rows.length} rows`;
    // Seen, not only in the DOM: opaque and with a box (an entrance that
    // started at opacity 0 left it invisible, r27 shots).
    const r = rows[0].getBoundingClientRect();
    if (getComputedStyle(rows[0]).opacity !== "1" || r.width === 0 || r.height === 0) return "invisible";
    const cls = rows[0].querySelector(".message-status")?.getAttribute("class") ?? "";
    return /message-status-([a-z_]+)/.exec(cls)?.[1] ?? "no status";
  }, text.slice(0, 40))) as string;
}

describe("our own message: the clock, one tick while the friend is away, two once they're back", function () {
  this.timeout(1_800_000);
  const alice = newUser("alice");
  const bob = newUser("bob");
  const text = `while you were away ${Date.now()}`;

  it("Alice and Bob are friends, and Bob logs out", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    const hello = `ticks ${Date.now()}`;
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, bob, hello);
    await friendRow(a, bob.displayName);
    await logOut(b, bob);
  });

  it("Alice's message to the away Bob reaches one tick (sent) and stays there, never back on the clock", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    await openChat(a, bob.displayName);
    await typeAndSend(a, ".message-input-field", text);
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 1500 });
    let state = "";
    await waitUntil(
      async () => {
        state = await ownState(a, text);
        return state === "sent";
      },
      NET_WAIT_MS,
      `Alice's own row never reached sent (last: ${state || "absent"})`,
    );
    await shot("sent-while-away");
    // Bob is away: no receipt can come, and nothing may put it back on the clock.
    for (let i = 0; i < 10; i++) {
      await sleep(1000);
      expect(await ownState(a, text)).toBe("sent");
    }
  });

  it("Bob unlocks, and Alice's message shows two ticks (delivered)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await unlock(b, bob);
    let state = "";
    await waitUntil(
      async () => {
        state = await ownState(a, text);
        return state === "delivered" || state === "read";
      },
      NET_WAIT_MS * 2,
      `Alice's own row never reached delivered after Bob came back (last: ${state})`,
    );
    await shot("delivered");
  });
});
