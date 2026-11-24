// Plan E6.11 and peer finding F5: two answer races the network gives only
// sometimes, provoked every run by wdio.races.conf.ts's held answers (each
// command's work is done, then its answer waits HELD_MS; webdriver.rs).
//
// F5: Bob accepts Alice. The spec arms Bob's next friends snapshot, so the
// one the accept asks for is read then (Alice not yet online) and held until
// the spec releases it, which it does only once FriendChanged has shown
// Alice online. Bob must keep showing her online after it lands (the
// snapshot keeps a held row's live presence). Event-gated, so it provokes
// the order on any machine (a fixed delay lost to a slow first beat).
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
import { existsSync, writeFileSync } from "node:fs";
import path from "node:path";
import { HELD_MS, RELEASE_DIR } from "../../wdio.races.conf.ts";

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
    // Arm Bob's next friends snapshot: the one the accept asks for is read
    // now and held until this spec releases it (REKINDLE_WDIO_RELEASE_DIR).
    const arm = path.join(RELEASE_DIR, "get_friends.arm");
    const release = path.join(RELEASE_DIR, "get_friends");
    writeFileSync(arm, "");
    const acceptedAt = Date.now();
    await acceptRequest(b, bob, hello);
    await friendRow(b, alice.displayName);
    // The accept's snapshot request was taken and is held (the arm consumed).
    await waitUntil(async () => !existsSync(arm), NET_WAIT_MS, "Bob's window asked for no friends snapshot after the accept: nothing held");
    // Alice's live status lands first, while the snapshot is held.
    let log: Step[] = [];
    await waitUntil(
      async () => (log = await steps(b, "alice-row")).some((x) => x.s === "online"),
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(`Bob's row for ${alice.displayName}: never online while the snapshot was held; ${told(log, acceptedAt)}`);
    });
    const online = log.find((x) => x.s === "online")!;
    await shot("f5-live");
    // Release the held snapshot (read before Alice was online) and see it land.
    writeFileSync(release, "");
    const releasedAt = Date.now();
    await waitUntil(async () => !existsSync(release), NET_WAIT_MS, "the held snapshot was never released");
    await sleep(3000);
    log = await steps(b, "alice-row");
    await shot("f5-after-late-answer");
    const back = log.filter((x) => x.t >= online.t).find((x) => x.s === "offline" || x.s === "absent");
    console.log(`[races] F5: ${told(log, acceptedAt)} (released at +${((releasedAt - acceptedAt) / 1000).toFixed(1)}s)`);
    if (back) {
      throw new Error(`Bob's row for ${alice.displayName}: reached online, then reverted to ${back.s} at +${((back.t - acceptedAt) / 1000).toFixed(1)}s (the late snapshot); ${told(log, acceptedAt)}`);
    }
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
