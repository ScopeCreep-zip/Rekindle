// Plan E6.11 and peer finding F5: two answer races the network gives only
// sometimes, provoked every run by wdio.races.conf.ts's held answers (each
// command's work is done, then its answer waits HELD_MS; webdriver.rs).
//
// F5: Bob accepts Alice. His window asks for the friends snapshot at once
// and the snapshot is read then (Alice not yet online), but it lands only
// HELD_MS later, after FriendChanged has shown Alice online. Bob must keep
// showing her online (the snapshot keeps a held row's live presence).
//
// Clock: Alice sends Bob a message. It commits (the row arrives queued),
// the outbox sends it and Bob's receipt marks it delivered, and only then
// does the send's commit-time answer land. Alice's row must not go back to
// queued (the pending clock).
//
// Each case records every state in the page (a MutationObserver, with the
// time), requires the race to have happened in that order (the live state
// before the held answer can land), and reports the sequence on failure.
// Each state is a shot.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, resolveIntent, sleep, typeAndSend, waitUntil } from "../../../walk/helpers.ts";
import { DOT_STATUS, acceptRequest, createIdentity, friendCode, friendRow, newUser, openChat, pendingRequest, requestFriend } from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { HELD_MS } from "../../wdio.races.conf.ts";

/** How long past a held answer's earliest landing the state is watched (the request may go out a little after the action). */
const SETTLE_MS = 12_000;

interface Step {
  s: string;
  t: number;
}

/** Start recording, in `b`'s page under `key`, each state of friend `name`'s buddy-list dot (DOT_STATUS values). */
async function recordFriend(b: Browser, key: string, name: string, labels: Record<string, string>): Promise<void> {
  await b.execute(
    (k: string, n: string, map: Record<string, string>) => {
      const w = window as unknown as { __steps?: Record<string, { s: string; t: number }[]> };
      w.__steps ??= {};
      const log: { s: string; t: number }[] = [];
      w.__steps[k] = log;
      const note = (): void => {
        const row = Array.from(document.querySelectorAll(".buddy-item")).find((r) => (r.querySelector(".buddy-name")?.textContent ?? "").includes(n));
        const label = row?.querySelector('[role="img"]')?.getAttribute("aria-label") ?? "";
        const s = row ? (map[label] ?? `"${label}"`) : "absent";
        if (log[log.length - 1]?.s !== s) log.push({ s, t: Date.now() });
      };
      note();
      new MutationObserver(note).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["class", "aria-label"] });
    },
    key,
    name,
    labels,
  );
}

/** Start recording, in `b`'s page under `key`, each delivery state of our own row holding `text`. */
async function recordOwnRow(b: Browser, key: string, text: string): Promise<void> {
  await b.execute(
    (k: string, t: string) => {
      const w = window as unknown as { __steps?: Record<string, { s: string; t: number }[]> };
      w.__steps ??= {};
      const log: { s: string; t: number }[] = [];
      w.__steps[k] = log;
      const note = (): void => {
        const row = Array.from(document.querySelectorAll(".chat-message")).find((m) => (m.textContent ?? "").includes(t));
        const cls = row?.querySelector(".message-status")?.getAttribute("class") ?? "";
        const s = row ? (/message-status-([a-z_]+)/.exec(cls)?.[1] ?? "no status") : "absent";
        if (log[log.length - 1]?.s !== s) log.push({ s, t: Date.now() });
      };
      note();
      new MutationObserver(note).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
    },
    key,
    text.slice(0, 40),
  );
}

async function steps(b: Browser, key: string): Promise<Step[]> {
  return (await b.execute(
    (k: string) => (window as unknown as { __steps?: Record<string, { s: string; t: number }[]> }).__steps?.[k] ?? [],
    key,
  )) as Step[];
}

/** `log` as "s@+1.2s → …", times from `from`. */
function told(log: Step[], from: number): string {
  return log.map((x) => `${x.s}@${((x.t - from) / 1000).toFixed(1)}s`).join(" → ");
}

/**
 * The race happened and the live state held: a `live` state was reached
 * before `from + HELD_MS - 1000` (so the held answer landed after it), and
 * nothing in `stale` followed it up to `from + HELD_MS + SETTLE_MS`.
 */
async function liveStateHeld(b: Browser, key: string, from: number, live: string[], stale: string[], what: string, tag: string): Promise<Step[]> {
  let log: Step[] = [];
  await waitUntil(
    async () => {
      log = await steps(b, key);
      return log.some((x) => live.includes(x.s));
    },
    NET_WAIT_MS,
    "",
  ).catch(() => {
    throw new Error(`${what}: never reached ${live.join("/")}; ${told(log, from)}`);
  });
  const reached = log.find((x) => live.includes(x.s))!;
  // The moment the live state is reached, before the held answer lands.
  await shot(`${tag}-live`);
  if (reached.t >= from + HELD_MS - 1000) {
    throw new Error(`${what}: the race was not provoked (${reached.s} only at +${((reached.t - from) / 1000).toFixed(1)}s, the held answer lands at +${HELD_MS / 1000}s); ${told(log, from)}`);
  }
  const until = from + HELD_MS + SETTLE_MS;
  while (Date.now() < until) await sleep(500);
  log = await steps(b, key);
  // After the held answer has landed: the shot either way, before any failure.
  await shot(`${tag}-after-late-answer`);
  const after = log.filter((x) => x.t >= reached.t);
  const back = after.find((x) => stale.includes(x.s));
  if (back) {
    throw new Error(`${what}: reached ${reached.s}, then reverted to ${back.s} at +${((back.t - from) / 1000).toFixed(1)}s (the held answer); ${told(log, from)}`);
  }
  return log;
}

describe("held answers: a late snapshot or send answer never undoes a live state", function () {
  this.timeout(1_800_000);
  const alice = newUser("alice");
  const bob = newUser("bob");
  const hello = `races ${Date.now()}`;

  it("Bob accepts Alice and keeps showing her online after his friends snapshot lands late (F5)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await recordFriend(b, "alice-row", alice.displayName, DOT_STATUS);
    const acceptedAt = Date.now();
    await acceptRequest(b, bob, hello);
    await friendRow(b, alice.displayName);
    const log = await liveStateHeld(b, "alice-row", acceptedAt, ["online"], ["offline", "absent"], `Bob's row for ${alice.displayName}`, "f5");
    console.log(`[races] F5: ${told(log, acceptedAt)}`);
    expect(log[log.length - 1].s).toBe("online");
  });

  it("Alice's message keeps its tick after the send's commit-time answer lands late (clock)", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const text = `held answer ${Date.now()}`;
    await openChat(a, bob.displayName);
    await recordOwnRow(a, "own-row", text);
    await typeAndSend(a, ".message-input-field", text);
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 1500 });
    const committedAt = Date.now();
    const log = await liveStateHeld(a, "own-row", committedAt, ["sent", "delivered", "read"], ["queued"], `Alice's own "${text.slice(0, 20)}"`, "clock");
    console.log(`[races] clock: ${told(log, committedAt)}`);
    expect(["sent", "delivered", "read"]).toContain(log[log.length - 1].s);
  });
});
