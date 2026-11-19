// Plan E7.14 S2: two users, each their own app, Host and Veilid node on
// this machine (multiremote, wdio.peers.conf.ts). Alice befriends Bob by
// friend code, Bob accepts, and a DM goes each way: friend handshake,
// PQXDH, the Double Ratchet, the outbox and Veilid private routes, all
// real.

import { multiRemoteBrowser } from "@wdio/globals";
import { acceptRequest, createIdentity, friendCode, friendRow, newUser, openChat, pendingRequest, requestFriend, seesMessage } from "../../journeys.ts";
import { resolveIntent, typeAndSend } from "../../../walk/helpers.ts";

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
  });

  it("lists each as the other's friend once Bob accepts", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await acceptRequest(b, bob, hello);
    await friendRow(b, alice.displayName);
    await friendRow(a, bob.displayName);
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
  });

  it("delivers Bob's reply to Alice", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const text = `dm b→a ${Date.now()}`;
    await typeAndSend(b, ".message-input-field", text);
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: 1500 });
    await seesMessage(b, bob, text);
    await seesMessage(a, alice, text);
  });
});
