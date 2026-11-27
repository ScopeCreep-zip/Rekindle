// FT1: the Host refuses the draft as one request, or creates the thread
// and accepts its starter. Cara supplies one denied and one writable
// forum, so the test uses actual governance instead of a mocked answer.
import { browser, expect } from "@wdio/globals";
import { byText, gone, NET_WAIT_MS, shown, waitUntil } from "../../../walk/helpers.ts";
import { setValue } from "../../../walk/nav.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerReviewed, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Forum Hall ${Date.now() % 100000}`;
const DENIED = "Read-only forum";
const WRITABLE = "Writable forum";
const TITLE = "A post with a starter";
const BODY = "The starter the Host accepted";
const COMPOSER = ".forum-channel-composer";
const CREATE = ".forum-channel-composer-actions .btn-primary";
const TITLE_INPUT = ".forum-channel-input";
const BODY_INPUT = ".forum-channel-body";
const THREAD_MESSAGE = ".thread-panel .chat-message";

/** How long the community key and then the starter's send may take: generous, so only a real stall fails. */
const KEY_WAIT_MS = 300_000;

/** CREATE_PUBLIC_THREADS (bit 45); VIEW_CHANNELS and SEND_IN_THREADS stay allowed. */
const CREATE_PUBLIC_THREADS = "0x200000000000";

async function createForum(owner: Peer, anchor: string, name: string): Promise<string> {
  const result = await peerReviewed(owner, ["channel", "create", "-c", anchor, "-n", name, "--kind", "forum"]);
  const id = (result as { id?: string } | null)?.id;
  if (!id || !/^[0-9a-f]{32}$/i.test(id)) throw new Error(`The Host did not return an id for ${name}`);
  return id;
}

async function openForum(name: string): Promise<void> {
  await (await byText(browser, ".channel-item", name, NET_WAIT_MS)).click();
  await byText(browser, ".forum-channel-title", name, NET_WAIT_MS);
}

async function composePost(): Promise<void> {
  await (await shown(browser, ".forum-channel-new-post-btn", NET_WAIT_MS)).click();
  await setValue(browser, TITLE_INPUT, TITLE);
  await setValue(browser, BODY_INPUT, BODY);
  await (await shown(browser, CREATE)).click();
}

async function starterRows(): Promise<string[]> {
  return (await browser.execute((selector: string, body: string) => (
    Array.from(document.querySelectorAll(selector))
      .filter((row) => (row.textContent ?? "").includes(body))
      .map((row) => row.getAttribute("data-message-id") ?? "")
  ), THREAD_MESSAGE, BODY)) as string[];
}

describe("forum post accepted as one request (FT1)", function () {
  this.timeout(2_400_000);

  const member: User = { ...newUser("ft1"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, member);
    peers = await startFleet(["Cara"]);
    const [cara] = peers;
    const anchor = await peerCreateCommunity(cara, COMMUNITY);
    const denied = await createForum(cara, anchor, DENIED);
    await createForum(cara, anchor, WRITABLE);
    await peerReviewed(cara, [
      "community", "channel", "overwrite", "set", "-c", anchor, "-C", denied,
      "--target", "everyone", "--deny", CREATE_PUBLIC_THREADS,
    ]);
    // Join after Cara has written the channels and overwrite, so the member's
    // first governance view includes the refusal without a grant/revoke race.
    await joinCommunity(browser, member, await peerInvite(cara, anchor), COMMUNITY);
    await openCommunity(browser, COMMUNITY);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("keeps the draft and shows the Host's reason when creating a post is refused", async () => {
    await openForum(DENIED);
    await composePost();
    const toast = await byText(browser, ".toast-error", "permission denied", NET_WAIT_MS);
    expect((await toast.getText()).toLowerCase()).toContain("permission denied");
    expect(await (await shown(browser, TITLE_INPUT)).getValue()).toBe(TITLE);
    expect(await (await shown(browser, BODY_INPUT)).getValue()).toBe(BODY);
    expect(await browser.$$(".forum-channel-thread-card")).toHaveLength(0);
    await shot("refused-draft");
    await (await shown(browser, ".forum-channel-composer-actions .btn-secondary")).click();
    await gone(browser, COMPOSER);
  });

  it("shows the accepted starter once, including after opening the thread again", async () => {
    await openForum(WRITABLE);
    await composePost();
    await gone(browser, COMPOSER, NET_WAIT_MS);
    await byText(browser, ".thread-panel-title", TITLE, NET_WAIT_MS);
    await byText(browser, THREAD_MESSAGE, BODY, NET_WAIT_MS);
    const [id] = await starterRows();
    expect(id ?? "").toMatch(/^tmsg_[0-9a-f]+$/i);
    expect(await starterRows()).toEqual([id]);
    await shot("accepted-starter");

    await (await shown(browser, '.thread-panel button[aria-label="Close thread"]')).click();
    await (await byText(browser, ".forum-channel-thread-card", TITLE, NET_WAIT_MS)).click();
    await byText(browser, ".thread-panel-title", TITLE, NET_WAIT_MS);
    await byText(browser, THREAD_MESSAGE, BODY, NET_WAIT_MS);
    expect(await starterRows()).toEqual([id]);
    await shot("starter-after-reopen");

    // Ari joined Cara's community, so the starter waits for the community
    // key from Cara (the "Waiting for the community key" banner) and stays
    // queued; once the key arrives the outbox writes it and its row turns
    // sent (FT1: ChannelSendStatus on the thread's own id). A banner that
    // never clears within this bound is a key-delivery finding.
    const keyFrom = Date.now();
    await waitUntil(async () => !(await browser.$(".key-pending-notice").isExisting()), KEY_WAIT_MS, "").catch(() => {
      throw new Error(`the community key never arrived: the key-pending banner still shows after ${KEY_WAIT_MS / 1000} s (a key-delivery finding)`);
    });
    console.log(`[forum] community key arrived after ${Math.round((Date.now() - keyFrom) / 1000)} s`);
    const sentFrom = Date.now();
    await waitUntil(
      async () => browser.$(`${THREAD_MESSAGE}[data-message-id="${id}"] .message-status-sent`).isExisting(),
      KEY_WAIT_MS,
      `the starter ${id} never turned sent after the key arrived`,
    );
    console.log(`[forum] starter sent ${Math.round((Date.now() - sentFrom) / 1000)} s after the key`);
    await shot("starter-sent");
  });
});
