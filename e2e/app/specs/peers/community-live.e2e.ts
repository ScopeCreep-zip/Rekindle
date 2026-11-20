// Plan E7.14 (Linux IB2, 2026-11-20): a community post reaches the other
// member live. The community window lost its `ChannelMessageEvent::New`
// handler when the desktop channel adapter was deleted (E3.1), so a post
// showed only after the other member reselected the channel; fixed in
// 3dbe61a8c. Two users, each their own app, Host and Veilid node
// (multiremote, wdio.peers.conf.ts): Alice creates a community and a
// channel, Bob joins by invite, both open the channel, and each post must
// appear in the other's open channel without a reselect, with no unread
// badge on the channel being read and no failed mark-read.

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, byText, waitUntil } from "../../../walk/helpers.ts";
import {
  type User,
  consoleSeen,
  createCommunity,
  createIdentity,
  createInvite,
  createTextChannel,
  joinCommunity,
  newUser,
  openCommunity,
  postMessage,
  selectChannel,
  watchConsole,
} from "../../journeys.ts";

/** How long a post may take to show in the other member's open channel. */
const LIVE_MS = 30_000;
const COMMUNITY = "Live Posts";
const CHANNEL = "general";

/** `b`'s open channel shows `text` within LIVE_MS, then no unread badge, and no failed mark-read. */
async function seesLive(b: Browser, text: string): Promise<void> {
  await byText(b, ".community-main .chat-message", text.slice(0, 40), LIVE_MS);
  // Viewing the channel marks it read (handleSelectChannel, and the Host's
  // count of a message in the viewed channel): the badge clears after that
  // round trip, if it shows at all.
  const channel = await byText(b, ".channel-item", CHANNEL);
  await waitUntil(
    async () => !(await (await channel.$(".channel-unread-badge")).isExisting()),
    WAIT_MS,
    `#${CHANNEL} still shows an unread badge while open`,
  );
  const seen = await consoleSeen(b);
  expect(seen).not.toBeNull();
  expect(seen.filter((line) => line.includes("Failed to mark channel read"))).toEqual([]);
}

/** `b` opens COMMUNITY's window on CHANNEL, reopening it until its history shows `text`. */
async function opensWithHistory(b: Browser, text: string): Promise<void> {
  const deadline = Date.now() + NET_WAIT_MS;
  for (;;) {
    await openCommunity(b, COMMUNITY);
    await selectChannel(b, CHANNEL);
    const shown = await byText(b, ".community-main .chat-message", text.slice(0, 40), WAIT_MS).then(
      () => true,
      () => false,
    );
    if (shown) return;
    if (Date.now() > deadline) throw new Error(`the channel's history never showed "${text}" in ${NET_WAIT_MS} ms of reopening`);
    await b.closeWindow();
  }
}

describe("a community post reaches the other member live", () => {
  const alice = newUser("alice");
  const bob = newUser("bob");

  it("creates an identity on each app", async () => {
    await Promise.all([
      createIdentity(multiRemoteBrowser.getInstance("alice"), alice),
      createIdentity(multiRemoteBrowser.getInstance("bob"), bob),
    ]);
  });

  it("Bob joins Alice's community by invite, and both open its channel", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await createCommunity(a, alice, COMMUNITY);
    await openCommunity(a, COMMUNITY);
    await createTextChannel(a, alice, CHANNEL);
    await selectChannel(a, CHANNEL);
    // A post from before Bob joins, which his channel must show from its
    // history before any live post: then that load is over, and what follows
    // can reach his list only by the live path under test. A load still in
    // flight can carry a live post in and hide a missing live path, and a
    // post stored after the load shows only by the live path (both seen with
    // the fix reverted at 3dbe61a8c). Opening the window loads the history;
    // reselecting the channel does not (selectedChannelId unchanged), and no
    // count marks posts from before the join, so until the post shows Bob
    // closes and reopens the window.
    const before = `before the join ${Date.now()}`;
    await postMessage(a, alice, before);
    const link = await createInvite(a, alice, 1);
    await joinCommunity(b, bob, link, COMMUNITY);
    await opensWithHistory(b, before);
    await Promise.all([watchConsole(a), watchConsole(b)]);
  });

  it("Alice's post shows in Bob's open channel", async () => {
    const text = `live a→b ${Date.now()}`;
    await postMessage(multiRemoteBrowser.getInstance("alice"), alice, text);
    await seesLive(multiRemoteBrowser.getInstance("bob"), text);
  });

  it("Bob's reply shows in Alice's open channel", async () => {
    const text = `live b→a ${Date.now()}`;
    await postMessage(multiRemoteBrowser.getInstance("bob"), bob, text);
    await seesLive(multiRemoteBrowser.getInstance("alice"), text);
  });
});
