// Plan E7.14: journeys between the app's user and fleet peers (fleet.ts),
// shared by the specs that need friends.

import type { Browser } from "webdriverio";
import { type Peer, peerRequestFriend } from "./fleet.ts";
import { type User, acceptRequest, friendCode, friendRow, pendingRequest } from "./journeys.ts";

/**
 * Each peer sends the user a friend request by the user's friend code and
 * the user accepts it in the buddy list. Returns the user's public key,
 * read from that code (`rekindle://add/<key>/…`).
 */
export async function befriendFleet(b: Browser, me: User, peers: Peer[]): Promise<string> {
  const code = await friendCode(b);
  const myKey = /^rekindle:\/\/add\/([0-9a-f]{64})\//.exec(code)?.[1];
  if (!myKey) throw new Error("the friend code names no public key");
  for (const p of peers) {
    const hello = `hello from ${p.publicKey.slice(0, 8)}`;
    await peerRequestFriend(p, code, hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, me, hello);
    await friendRow(b, p.displayName.slice(0, 24));
  }
  return myKey;
}
