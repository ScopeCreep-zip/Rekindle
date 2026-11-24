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
  channelCount,
  consoleSeen,
  createCommunity,
  createIdentity,
  createInvite,
  joinCommunity,
  newUser,
  openCommunity,
  postMessage,
  selectChannel,
  setCommunityName,
  watchConsole,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";

/** How long a post may take to show in the other member's open channel. */
const LIVE_MS = 30_000;
const COMMUNITY = "Live Posts";
const CHANNEL = "general";
/** The names each sets in the community (none is shown until set). */
const ALICE_NAME = "Alice Live";
const BOB_NAME = "Bob Live";

/** `b`'s member list shows a member named `name`. */
async function membersShow(b: Browser, name: string): Promise<void> {
  await waitUntil(
    async () =>
      (await b.execute(
        (n: string) => Array.from(document.querySelectorAll(".member-item .member-name")).some((e) => (e.textContent ?? "").trim() === n),
        name,
      )) as boolean,
    NET_WAIT_MS,
    `the member list never showed ${name}`,
  );
}

/**
 * `b`'s open channel shows `text` within LIVE_MS under the sender name
 * `sender` (the sender's community name), then no unread badge, and no failed
 * mark-read.
 */
/**
 * Presence in `b`'s open community window while both members view CHANNEL
 * (the lead, 2026-11-23: Bob's list put a posting Alice under Offline, and
 * his own row showed Away while Online): `other` is in the Online group;
 * `self`'s own row reads Online, as their buddy list does (both stay Online
 * throughout); and CHANNEL's presence badge counts both members there
 * ("2 here", ChannelList.tsx channel-presence-badge, members by location).
 */
async function presenceRight(b: Browser, self: string, other: string): Promise<void> {
  let last = "";
  await waitUntil(
    async () => {
      last = (await b.execute(
        (me: string, them: string, channel: string) => {
          const groups: Record<string, string> = {};
          let group = "";
          let mine = "";
          for (const el of Array.from(document.querySelectorAll(".member-group-header, .member-item"))) {
            if (el.classList.contains("member-group-header")) {
              group = (el.textContent ?? "").trim();
              continue;
            }
            const name = (el.querySelector(".member-name")?.textContent ?? "").trim();
            groups[name] = group;
            if (name === me) mine = el.querySelector('[role="img"]')?.getAttribute("aria-label") ?? "";
          }
          const row = Array.from(document.querySelectorAll(".channel-item")).find((r) => (r.querySelector(".channel-name")?.textContent ?? "").trim() === channel);
          const here = row?.querySelector(".channel-presence-badge")?.getAttribute("title") ?? "no presence badge";
          const faults: string[] = [];
          if (!/^online/i.test(groups[them] ?? "")) faults.push(`${them} under "${groups[them] ?? "no group"}"`);
          if (!/^online$/i.test(mine)) faults.push(`own row reads "${mine || "nothing"}"`);
          if (here !== "2 here") faults.push(`#${channel} badge "${here}"`);
          return faults.join("; ");
        },
        self,
        other,
        CHANNEL,
      )) as string;
      return last === "";
    },
    NET_WAIT_MS,
    `presence in ${self}'s community window is wrong`,
  ).catch(() => {
    throw new Error(`presence in ${self}'s community window: ${last}`);
  });
}

async function seesLive(b: Browser, text: string, sender: string): Promise<void> {
  const message = await byText(b, ".community-main .chat-message", text.slice(0, 40), LIVE_MS);
  await waitUntil(
    async () =>
      (await b.execute(
        (el: HTMLElement) => el.closest(".chat-message")?.querySelector(".chat-message-sender-other")?.textContent?.trim() ?? "",
        message as unknown as HTMLElement,
      )) === sender,
    NET_WAIT_MS,
    `"${text.slice(0, 24)}" never showed under ${sender}`,
  );
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
    // The genesis #general (origin.rs: every community starts with one), never
    // a second channel of the same name.
    await waitUntil(async () => (await channelCount(a, CHANNEL)) === 1, NET_WAIT_MS, `the new community did not list one #${CHANNEL}`);
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
    expect(await channelCount(b, CHANNEL)).toBe(1);
    await Promise.all([watchConsole(a), watchConsole(b)]);
  });

  // A member has no name in a community until they set one (privacy by
  // design: member_card.rs); each sets theirs, and the other's member list
  // shows it (the member card's propagation, plan S6/S6c).
  it("each sets a name in the community, and the other's member list shows it", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await setCommunityName(a, alice, ALICE_NAME);
    await setCommunityName(b, bob, BOB_NAME);
    await membersShow(b, ALICE_NAME);
    await membersShow(a, BOB_NAME);
    await shot("member-names");
  });

  it("Alice's post shows in Bob's open channel, under her community name", async () => {
    const text = `live a→b ${Date.now()}`;
    await postMessage(multiRemoteBrowser.getInstance("alice"), alice, text);
    await seesLive(multiRemoteBrowser.getInstance("bob"), text, ALICE_NAME);
    await presenceRight(multiRemoteBrowser.getInstance("bob"), BOB_NAME, ALICE_NAME);
    await shot("alice-post-at-bob");
  });

  it("Bob's reply shows in Alice's open channel, under his community name", async () => {
    const text = `live b→a ${Date.now()}`;
    await postMessage(multiRemoteBrowser.getInstance("bob"), bob, text);
    await seesLive(multiRemoteBrowser.getInstance("alice"), text, BOB_NAME);
    await presenceRight(multiRemoteBrowser.getInstance("alice"), ALICE_NAME, BOB_NAME);
    await shot("bob-post-at-alice");
  });
});
