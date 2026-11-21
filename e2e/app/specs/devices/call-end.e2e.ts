// Plan E7.14 (round 18's CE1, 60fb63d42): a 1:1 call ends cleanly when the
// peer is unreachable, the stuck peer can be called again, and a third
// friend calling during a call is told busy. Three users, each their own app,
// Host and Veilid node (wdio.devices-peers.conf.ts: alice, bob, carol), each
// needing the microphone: run with `pnpm test:app:devices` on Linux, or on
// macOS from the owner's own terminal (TCC refuses an app a Claude session
// launched; the suite never raises a permission prompt).
//
// - Busy (CE1 (c)): while alice and bob are in a call, carol calls alice;
//   alice's Host declines "busy" and carol is told "Call declined: busy".
// - Hang up with the route down (CE1 (d), (a)): bob's app is frozen
//   (SIGSTOP: no packet, no answer), alice hangs up from the popped-out call
//   window, and the window closes at once: the call ends locally first and
//   its CallEnd waits in the outbox.
// - Re-call the stuck peer (CE1 (c)): bob is resumed (SIGCONT); alice calls
//   again and bob's incoming-call modal rings, whether bob's Host has by then
//   taken the queued CallEnd or still holds the old call (a stale call with
//   the same peer is superseded before ringing).
// The freeze stops the whole of bob's app (its Host and node run in it); it
// is a process this suite launched, found by the port its driver listens on.

import { execFileSync } from "node:child_process";
import { multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, press, shown, switchTo, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { acceptRequest, createIdentity, friendCode, friendRow, newUser, openChat, pendingRequest, requestFriend, type User } from "../../journeys.ts";

const app = (name: "alice" | "bob" | "carol"): Browser => multiRemoteBrowser.getInstance(name) as unknown as Browser;

/** The pid of the app whose driver listens on `b`'s port. */
function pidOf(b: Browser): number {
  const port = b.options.port;
  if (!port) throw new Error("the session has no port");
  const out = execFileSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"], { encoding: "utf8" }).trim().split("\n")[0];
  const pid = Number(out);
  if (!Number.isInteger(pid) || pid <= 0) throw new Error(`no process listens on port ${port}`);
  return pid;
}

/** The toasts `b`'s buddy list shows. */
async function toasts(b: Browser): Promise<string[]> {
  await switchTo(b, BUDDY);
  return (await b.execute(() => Array.from(document.querySelectorAll(".toast-item")).map((t) => (t.textContent ?? "").trim()))) as string[];
}

/** What `b`'s current window says when a call does not start: its alerts, statuses and toasts. */
async function windowSays(b: Browser): Promise<string> {
  return (await b.execute(() =>
    Array.from(document.querySelectorAll("[role=alert], [role=status], .toast-item"))
      .map((el) => el.textContent?.trim() ?? "")
      .filter(Boolean)
      .join(" | "),
  )) as string;
}

/** In the DM window `b` is on, start a voice call with `peer`. */
async function call(b: Browser, peer: User): Promise<void> {
  await press(b, await shown(b, `[aria-label="Start voice call with ${peer.displayName}"]`, NET_WAIT_MS));
}

/** `b`'s buddy list shows the incoming-call modal from `from`. */
async function rings(b: Browser, from: User, timeout = NET_WAIT_MS): Promise<void> {
  await switchTo(b, BUDDY);
  const modal = await shown(b, ".incoming-call-modal", timeout).catch(async () => {
    throw new Error(`no incoming call from ${from.displayName} rang; the buddy list says: ${(await windowSays(b)) || "nothing"}`);
  });
  await waitUntil(async () => (await modal.getText()).includes(from.displayName), WAIT_MS, `the ringing call is not from ${from.displayName}`);
}

/** In the DM window `b` is on, the call with its peer is connected (ActiveDmCallPanel shows only an active call). */
async function connected(b: Browser): Promise<void> {
  await shown(b, ".active-call-panel", NET_WAIT_MS).catch(async () => {
    throw new Error(`the call never connected; the window says: ${(await windowSays(b)) || "nothing"}`);
  });
}

describe("a call ends with the peer unreachable, the peer can be called again, a third caller hears busy", function () {
  this.timeout(1_800_000);
  const alice = newUser("alice");
  const bob = newUser("bob");
  const carol = newUser("carol");
  let bobPid = 0;

  after(() => {
    // Never leave bob's app frozen.
    if (bobPid) {
      try {
        process.kill(bobPid, "SIGCONT");
      } catch {
        /* already gone */
      }
    }
  });

  it("creates the three users; bob and carol are alice's friends", async () => {
    await Promise.all([createIdentity(app("alice"), alice), createIdentity(app("bob"), bob), createIdentity(app("carol"), carol)]);
    for (const [b, user] of [[app("bob"), bob], [app("carol"), carol]] as const) {
      const hello = `call test from ${user.displayName}`;
      await requestFriend(b, user, await friendCode(app("alice")), hello);
      await pendingRequest(app("alice"), hello);
      await acceptRequest(app("alice"), alice, hello);
      await friendRow(app("alice"), user.displayName);
      await friendRow(b, alice.displayName);
    }
  });

  it("alice calls bob and bob answers", async () => {
    await openChat(app("bob"), alice.displayName);
    await openChat(app("alice"), bob.displayName);
    await call(app("alice"), bob);
    await rings(app("bob"), alice);
    await press(app("bob"), await shown(app("bob"), ".incoming-call-accept"));
    await connected(app("alice"));
    await waitForHandle(app("bob"), (h) => h.startsWith("chat-"), WAIT_MS);
  });

  it("carol calls alice during the call and is told busy", async () => {
    await openChat(app("carol"), alice.displayName);
    await call(app("carol"), alice);
    await waitUntil(async () => (await toasts(app("carol"))).some((t) => t.includes("Call declined: busy")), NET_WAIT_MS, "carol was never told busy");
    // Alice's call with bob is untouched.
    await switchTo(app("alice"), (await app("alice").getWindowHandles()).find((h) => h.startsWith("chat-")) ?? "");
    await connected(app("alice"));
  });

  it("with bob's app frozen, alice hangs up from the call window and it closes at once", async () => {
    const a = app("alice");
    const open = await a.getWindowHandles();
    await press(a, await shown(a, '[aria-label="Pop out call"]'));
    await a.switchToWindow(await waitForHandle(a, (h) => h.startsWith("call-") && !open.includes(h), WAIT_MS));
    bobPid = pidOf(app("bob"));
    process.kill(bobPid, "SIGSTOP");
    await press(a, await shown(a, '[aria-label="Hang up"]'));
    // Local first (CE1 (d)): no wait on the frozen peer.
    await waitUntil(async () => !(await a.getWindowHandles()).some((h) => h.startsWith("call-")), WAIT_MS, "the call window stayed open after Hang up with the peer unreachable");
  });

  it("bob, resumed, rings when alice calls again", async () => {
    process.kill(bobPid, "SIGCONT");
    bobPid = 0;
    const a = app("alice");
    await switchTo(a, (await a.getWindowHandles()).find((h) => h.startsWith("chat-")) ?? "");
    await call(a, bob);
    await rings(app("bob"), alice);
    await press(app("bob"), await shown(app("bob"), ".incoming-call-decline"));
  });
});
