// Plan E7.14 S4: the windows whose rows are other people, over a real peer
// fleet (fleet.ts, decided 2026-11-19): two headless peers with
// 64-character names befriend the app's user, one sends a DM, then the
// buddy list (friend rows and group headers), the DM window, the friend's
// profile window, a group chat of the three (E2.6: 3–8 members) and a
// community the peers joined by invite (its member list) are checked at
// each size.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, doubleClick, press, shown, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAcceptGroup, peerJoinCommunity, peerSendDm, peerSendGroup, startFleet, stopFleet } from "../../fleet.ts";
import {
  createCommunity,
  createGroup,
  createIdentity,
  createInvite,
  friendRow,
  newUser,
  openCommunity,
  openGroup,
  seesMessage,
  type User,
} from "../../journeys.ts";
import { befriendFleet } from "../../social.ts";
import { BUDDY_LIST, CHAT_DM, CHAT_GROUP, COMMUNITY, PROFILE, checkWindow } from "../../windows.ts";

/** A name of exactly 64 characters with no spaces (MAX_DISPLAY_NAME_LEN). */
function longName(prefix: string): string {
  return (prefix + "W".repeat(64)).slice(0, 64);
}

const LONG_URL = `https://example.com/${"averyveryverylongpathsegment".repeat(12)}?q=${"x".repeat(80)}`;

describe("layout of the windows about other people", () => {
  const me: User = { ...newUser("layout"), displayName: longName("Me_") };
  let peers: Peer[] = [];
  /** The user's public key, read from their friend code (`rekindle://add/<key>/…`). */
  let myKey = "";

  after(async () => {
    await stopFleet(peers);
  });

  it("befriends two real peers", async () => {
    await createIdentity(browser, me);
    peers = await startFleet([longName("PeerOne_"), longName("PeerTwo_")]);
    myKey = await befriendFleet(browser, me, peers);
  }).timeout(900_000);

  it("buddy list with friends", async () => {
    // A DM from a friend: the unread badge on their row, and the long URL for the DM window.
    await peerSendDm(peers[0], myKey, LONG_URL);
    await expect(await checkWindow(browser, BUDDY_LIST, { withData: true })).toEqual({});
  });

  it("DM window with a long unbroken URL from the peer", async () => {
    const before = await browser.getWindowHandles();
    await doubleClick(browser, await friendRow(browser, peers[0].displayName.slice(0, 24)));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h)));
    await shown(browser, ".message-input-field");
    await seesMessage(browser, me, LONG_URL.slice(0, 40));
    await expect(await checkWindow(browser, CHAT_DM, { withData: true })).toEqual({});
    await browser.closeWindow();
  });

  it("friend's profile window", async () => {
    const row = await friendRow(browser, peers[1].displayName.slice(0, 24));
    await browser.execute((node: HTMLElement) => {
      const r = node.getBoundingClientRect();
      node.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: r.x + 5, clientY: r.y + 5 }));
    }, row as unknown as HTMLElement);
    await press(browser, await byText(browser, ".context-menu-item", "View Profile"));
    await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("profile-")));
    await shown(browser, ".profile-actions");
    await expect(await checkWindow(browser, PROFILE, { withData: true })).toEqual({});
    await browser.closeWindow();
  });

  it("group chat of the user and both peers", async () => {
    const group = longName("Group_");
    await createGroup(browser, me, group, peers.map((p) => p.displayName));
    const id = await peerAcceptGroup(peers[0], group);
    await peerAcceptGroup(peers[1], group);
    await peerSendGroup(peers[0], id, LONG_URL);
    await openGroup(browser, group);
    await seesMessage(browser, me, LONG_URL.slice(0, 40));
    await expect(await checkWindow(browser, CHAT_GROUP, { withData: true })).toEqual({});
    await browser.closeWindow();
  }).timeout(900_000);

  it("community window whose member list holds the peers", async () => {
    const community = (`Community_${"W".repeat(100)}`).slice(0, 100);
    await createCommunity(browser, me, community);
    await openCommunity(browser, community);
    const invite = await createInvite(browser, me, peers.length);
    for (const p of peers) await peerJoinCommunity(p, invite);
    // The owner plus each peer, once the joins have merged into the owner's roster.
    await waitUntil(async () => (await browser.$$(".member-item")).length >= 1 + peers.length, NET_WAIT_MS, "the member list never held the peers");
    await expect(await checkWindow(browser, COMMUNITY, { withData: true })).toEqual({});
  }).timeout(900_000);
});
