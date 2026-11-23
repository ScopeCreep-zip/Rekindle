// Plan E7.14 S2: two users, each their own app, Host and Veilid node on
// this machine (multiremote, wdio.peers.conf.ts). Alice befriends Bob by
// friend code, Bob accepts, and a DM goes each way: friend handshake,
// PQXDH, the Double Ratchet, the outbox and Veilid private routes, all
// real. Each sender's own row is the Host's committed one, shown once,
// and its clock reaches Delivered (plan E6.11); each state is a shot.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { acceptRequest, createIdentity, friendCode, friendRow, newUser, openChat, pendingRequest, requestFriend, seesMessage } from "../../journeys.ts";
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
        return rows[0].querySelector(".message-status")?.getAttribute("class") ?? "no status";
      }, text.slice(0, 40))) as string;
      return /message-status-(delivered|read)\b/.test(seen);
    },
    NET_WAIT_MS,
    `the sender's own "${text.slice(0, 24)}" never showed once as delivered (last: ${seen})`,
  );
  expect(seen).toMatch(/message-status-(delivered|read)/);
}

describe("friend request, then a DM each way", () => {
  const alice = newUser("alice");
  const bob = newUser("bob");
  const hello = `hi bob, from ${alice.displayName}`;

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
    await shot("friends");
  });

  it("delivers a DM from Alice to Bob", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const text = `dm a→b ${Date.now()}`;
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
    await shot("dm-b-to-a");
  });
});
