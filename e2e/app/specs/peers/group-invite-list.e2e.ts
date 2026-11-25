// Plan E7.14 (lead, GV8 regression on Linux): a group invite reaches the
// invited user's buddy list. Bob's Host received the invite, yet his buddy
// list showed no Group Chats section, no "Accept invite to …", and both
// friends Offline. This spec runs that setup without voice (no microphone,
// so no TCC prompt on macOS) and requires, on Bob's buddy list: Alice
// shown online, the Group Chats section with the invite to accept, and no
// IPC refusal ("not allowed") or unhandled error in the window's console
// from the moment it opened. What the window logged is printed as
// `[gi] console:` either way, and each state is shot.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, shown, waitUntil } from "../../../walk/helpers.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { type Peer, startFleet, stopFleet } from "../../fleet.ts";
import {
  acceptRequest,
  createGroup,
  createIdentity,
  friendCode,
  friendRow,
  newUser,
  pendingRequest,
  requestFriend,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { befriendFleet } from "../../social.ts";

const GROUP = "Invite Check";

/**
 * Record, in the window `b` is on, every console warning and error and
 * every unhandled error or promise rejection (an invoke the ACL refuses
 * rejects with "… not allowed …"), from now on.
 */
async function watchAll(b: Browser): Promise<void> {
  await b.execute(`if (!window.__giLog) {
  window.__giLog = [];
  const text = (a) => (a instanceof Error ? a.message : typeof a === "string" ? a : JSON.stringify(a));
  for (const level of ["warn", "error"]) {
    const original = console[level].bind(console);
    console[level] = (...args) => { window.__giLog.push(level + ": " + args.map(text).join(" ")); original(...args); };
  }
  window.addEventListener("error", (e) => window.__giLog.push("uncaught: " + text(e.error ?? e.message)));
  window.addEventListener("unhandledrejection", (e) => window.__giLog.push("unhandled rejection: " + text(e.reason)));
}`);
}

async function seen(b: Browser): Promise<string[]> {
  return ((await b.execute("return window.__giLog || [];")) as string[]) ?? [];
}

/** Alice's row on `b`'s buddy list, with the status its dot names. */
async function aliceStatus(b: Browser, name: string): Promise<string> {
  return (await b.execute((n: string) => {
    const row = Array.from(document.querySelectorAll(".buddy-item")).find((r) => (r.textContent ?? "").includes(n));
    return row?.querySelector("[role=img]")?.getAttribute("aria-label") ?? "no row";
  }, name)) as string;
}

describe("a group invite reaches the invited buddy list (GV8 regression)", function () {
  this.timeout(1_800_000);
  const alice = newUser("alice");
  const bob = newUser("bob");
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Bob's buddy list shows Alice online, the Group Chats section and the invite, with no refused IPC", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    await inBuddy(b);
    await watchAll(b);
    const hello = `invite list friends ${Date.now()}`;
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, bob, hello);
    await friendRow(a, bob.displayName);
    await friendRow(b, alice.displayName);
    peers = await startFleet(["Cara"]);
    await befriendFleet(a, alice, peers);
    await createGroup(a, alice, GROUP, [bob.displayName, "Cara"]);

    await inBuddy(b);
    let invite = false;
    await waitUntil(
      async () => (invite = await (await b.$(`[aria-label="Accept invite to ${GROUP}"]`)).isDisplayed().catch(() => false)),
      NET_WAIT_MS,
      "",
    ).catch(() => undefined);
    const state = {
      invite,
      groupChats: (await b.execute(() =>
        Array.from(document.querySelectorAll(".pending-header")).some((h) => (h.textContent ?? "").includes("Group Chats")),
      )) as boolean,
      alice: await aliceStatus(b, alice.displayName),
    };
    const log = await seen(b);
    console.log(`[gi] bob's buddy list: ${JSON.stringify(state)}`);
    console.log(`[gi] console: ${JSON.stringify(log)}`);
    await shot("bob-buddy-list");
    expect(log.filter((l) => /not allowed|unhandled|uncaught/i.test(l))).toEqual([]);
    expect(state.groupChats).toBe(true);
    expect(state.invite).toBe(true);
    expect(state.alice.toLowerCase()).not.toContain("offline");
    await (await shown(b, `[aria-label="Accept invite to ${GROUP}"]`)).isDisplayed();
  });
});
