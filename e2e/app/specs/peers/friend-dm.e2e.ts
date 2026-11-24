// Plan E7.14 S2: two users, each their own app, Host and Veilid node on
// this machine (multiremote, wdio.peers.conf.ts). Alice befriends Bob by
// friend code, Bob accepts, and a DM goes each way: friend handshake,
// PQXDH, the Double Ratchet, the outbox and Veilid private routes, all
// real. Each sender's own row is the Host's committed one, shown once,
// and its clock reaches Delivered (plan E6.11); each state is a shot.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { DOT_STATUS, acceptRequest, createIdentity, friendCode, friendRow, newUser, openChat, ownStatusValue, pendingRequest, requestFriend, seesMessage } from "../../journeys.ts";
import { NET_WAIT_MS, resolveIntent, typeAndSend, waitUntil } from "../../../walk/helpers.ts";
import { shot } from "../../shots.ts";

/**
 * The sender's own row for `text` (plan E6.11: the Host's committed row, no
 * optimistic copy): exactly one in `b`'s open chat, and its clock reaches
 * Delivered (or Read) once the peer's receipt lands.
 */
async function ownRowDelivered(b: Browser, text: string): Promise<void> {
  let seen = "";
  await waitUntil(
    async () => {
      seen = (await b.execute((t: string) => {
        const rows = Array.from(document.querySelectorAll(".chat-message")).filter((m) => (m.textContent ?? "").includes(t));
        if (rows.length !== 1) return `${rows.length} rows`;
        // Seen, not only in the DOM: opaque and with a box.
        const r = rows[0].getBoundingClientRect();
        if (getComputedStyle(rows[0]).opacity !== "1" || r.width === 0 || r.height === 0) return "invisible";
        return rows[0].querySelector(".message-status")?.getAttribute("class") ?? "no status";
      }, text.slice(0, 40))) as string;
      return /message-status-(delivered|read)\b/.test(seen);
    },
    NET_WAIT_MS,
    `the sender's own "${text.slice(0, 24)}" never showed once as delivered (last: ${seen})`,
  );
  expect(seen).toMatch(/message-status-(delivered|read)/);
}

/**
 * `b`'s buddy list shows friend `name` with the status `them`'s own status
 * line shows (the lead, 2026-11-23: a friend saw Away while the user's own
 * line read Online; an idle Mac may legitimately go away, so the check is
 * that both agree, not a literal Online), and not as offline.
 */
async function friendStatusAgrees(b: Browser, name: string, them: Browser): Promise<void> {
  let last = "";
  await waitUntil(
    async () => {
      const theirs = await ownStatusValue(them);
      const dot = (await b.execute(
        (n: string) =>
          Array.from(document.querySelectorAll(".buddy-item"))
            .find((r) => (r.querySelector(".buddy-name")?.textContent ?? "").includes(n))
            ?.querySelector('[role="img"]')
            ?.getAttribute("aria-label") ?? "no row",
        name,
      )) as string;
      const seen = DOT_STATUS[dot] ?? dot;
      last = `${name} shows ${seen}, their own line ${theirs}`;
      return seen === theirs && seen !== "offline";
    },
    NET_WAIT_MS,
    `${name}'s status`,
  ).catch(() => {
    throw new Error(`friend status disagrees: ${last}`);
  });
}

describe("friend request, then a DM each way", () => {
  const alice = newUser("alice");
  const bob = newUser("bob");
  const hello = `hi bob, from ${alice.displayName}`;
  /** Alice's first DM, which must stay delivered after Bob's reply lands. */
  let first = "";

  it("creates an identity on each app", async () => {
    await Promise.all([
      createIdentity(multiRemoteBrowser.getInstance("alice"), alice),
      createIdentity(multiRemoteBrowser.getInstance("bob"), bob),
    ]);
  });

  it("delivers Alice's friend request to Bob", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await shot("request-pending");
  });

  it("lists each as the other's friend once Bob accepts", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await acceptRequest(b, bob, hello);
    await friendRow(b, alice.displayName);
    await friendRow(a, bob.displayName);
    // Each shows the other online, not only listed (peer finding F5: a
    // friends snapshot folded over the live presence put a friend back
    // offline).
    await friendStatusAgrees(a, bob.displayName, b);
    await friendStatusAgrees(b, alice.displayName, a);
    await shot("friends");
  });

  it("delivers a DM from Alice to Bob", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const text = `dm a→b ${Date.now()}`;
    first = text;
    await openChat(a, bob.displayName);
    await typeAndSend(a, ".message-input-field", text);
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 1500 });
    await seesMessage(a, alice, text);
    await openChat(b, alice.displayName);
    await seesMessage(b, bob, text);
    await ownRowDelivered(a, text);
    await shot("dm-a-to-b");
  });

  it("delivers Bob's reply to Alice", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const text = `dm b→a ${Date.now()}`;
    await typeAndSend(b, ".message-input-field", text);
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: 1500 });
    await seesMessage(b, bob, text);
    await seesMessage(a, alice, text);
    await ownRowDelivered(b, text);
    // Alice's own first message stays delivered (or read) and seen after
    // Bob's reply lands: the history load an incoming message started used
    // to fold delivered back to one tick (transformers.ts ownStatus).
    await ownRowDelivered(a, first);
    await shot("first-still-delivered");
    await shot("dm-b-to-a");
  });
});
