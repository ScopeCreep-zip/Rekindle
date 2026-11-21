// Plan E7.14 S5 (moved from E7.3's e2e/chat-announcer.spec.ts; wiring guide
// §13b, WCAG 2.2 SC 4.1.3 Status Messages): a chat window reads out, through
// its polite live region (ChatAnnouncer.tsx), each message a friend or group
// member sends while it is open, without moving focus. Stored history and
// the user's own sends are not read out; the message list itself is not
// live, so loading history is silent. The friends are real peers (fleet.ts):
// their messages come over Veilid, the outbox and the Double Ratchet, or the
// group's member chains (E2.6), never an injected event.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, doubleClick, resolveIntent, shown, typeAndSend, waitForHandle, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAcceptGroup, peerSendDm, peerSendGroup, startFleet, stopFleet } from "../../fleet.ts";
import { createGroup, createIdentity, friendRow, newUser, openGroup, seesMessage, type User } from "../../journeys.ts";
import { befriendFleet } from "../../social.ts";

const COMPOSER = ".message-input-field";

/** The lines the open chat window's announcer holds, in order. */
async function announced(): Promise<string[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll(".chat-announcer p")).map((p) => (p.textContent ?? "").trim()),
  )) as string[];
}

/** The announcer and the message list carry the live-region attributes §13b asks for. */
async function regionsAreRight(): Promise<void> {
  const attrs = (await browser.execute(() => {
    const a = document.querySelector(".chat-announcer");
    const list = document.querySelector(".chat-message-area");
    return {
      live: a?.getAttribute("aria-live") ?? null,
      relevant: a?.getAttribute("aria-relevant") ?? null,
      listLive: list?.getAttribute("aria-live") ?? null,
    };
  })) as { live: string | null; relevant: string | null; listLive: string | null };
  expect(attrs).toEqual({ live: "polite", relevant: "additions", listLive: "off" });
}

/** Focus the composer, as a user about to type does. */
async function focusComposer(): Promise<void> {
  const el = await shown(browser, COMPOSER, NET_WAIT_MS);
  await browser.execute((node: HTMLElement) => node.focus(), el as unknown as HTMLElement);
}

/** Whether the composer still holds focus. */
async function composerFocused(): Promise<boolean> {
  return (await browser.execute((sel: string) => document.activeElement === document.querySelector(sel), COMPOSER)) as boolean;
}

/** The regex for one announcement: "<sender> at <time>: <body>" (message_announcement.ts). */
function line(sender: string, body: string): RegExp {
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^${esc(sender)} at .+: ${esc(body)}$`);
}

/** Open the 1:1 chat with `friend` from the buddy list and switch to it. */
async function openDm(friend: Peer): Promise<void> {
  const before = await browser.getWindowHandles();
  await doubleClick(browser, await friendRow(browser, friend.displayName));
  await browser.switchToWindow(await waitForHandle(browser, (h) => h.startsWith("chat-") && !before.includes(h), NET_WAIT_MS));
  await shown(browser, COMPOSER, NET_WAIT_MS);
}

describe("a chat window announces new messages from others, not history or our own", function () {
  this.timeout(900_000);
  const me: User = newUser("announce");
  let peers: Peer[] = [];
  let myKey = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["AnnouncerPeerOne", "AnnouncerPeerTwo"]);
    myKey = await befriendFleet(browser, me, peers);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("the 1:1 chat announces the friend's new message, not history or our own send, and keeps focus", async () => {
    const friend = peers[0];
    const stored = `stored line ${Date.now()}`;
    await peerSendDm(friend, myKey, stored);
    // The CLI returns once the line is sent, not once it has arrived; seen
    // once and the window reopened, it is certainly stored history.
    await openDm(friend);
    await seesMessage(browser, me, stored);
    await browser.closeWindow();
    await openDm(friend);
    await seesMessage(browser, me, stored);
    await regionsAreRight();
    expect(await announced()).toEqual([]);

    const own = `sent by me ${Date.now()}`;
    await typeAndSend(browser, COMPOSER, own);
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await seesMessage(browser, me, own);
    expect(await announced()).toEqual([]);

    await focusComposer();
    const fresh = `gg wp ${Date.now()}`;
    await peerSendDm(friend, myKey, fresh);
    await seesMessage(browser, me, fresh);
    await waitUntil(async () => (await announced()).length > 0, NET_WAIT_MS, "the friend's new message was never announced");
    const lines = await announced();
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatch(line(friend.displayName, fresh));
    expect(await composerFocused()).toBe(true);
    await browser.closeWindow();
  });

  it("the group chat announces a member's new message, not the group's history", async () => {
    const group = `Announced ${Date.now()}`;
    await createGroup(browser, me, group, peers.map((p) => p.displayName));
    const id = await peerAcceptGroup(peers[0], group);
    await peerAcceptGroup(peers[1], group);
    const stored = `stored group line ${Date.now()}`;
    await peerSendGroup(peers[0], id, stored);
    // Seen once and the window reopened: certainly stored history.
    await openGroup(browser, group);
    await seesMessage(browser, me, stored);
    await browser.closeWindow();
    await openGroup(browser, group);
    await seesMessage(browser, me, stored);
    await regionsAreRight();
    expect(await announced()).toEqual([]);

    await focusComposer();
    const fresh = `hello group ${Date.now()}`;
    await peerSendGroup(peers[1], id, fresh);
    await seesMessage(browser, me, fresh);
    await waitUntil(async () => (await announced()).length > 0, NET_WAIT_MS, "the member's new message was never announced");
    const lines = await announced();
    expect(lines).toHaveLength(1);
    expect(lines[0]).toMatch(line(peers[1].displayName, fresh));
    expect(await composerFocused()).toBe(true);
    await browser.closeWindow();
  });
});
