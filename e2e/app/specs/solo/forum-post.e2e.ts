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
/** VIEW_CHANNELS (bit 0). */
const VIEW_CHANNELS = "0x1";

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
  let anchor = "";
  let writable = "";
  let deniedId = "";

  before(async () => {
    await createIdentity(browser, member);
    peers = await startFleet(["Cara"]);
    const [cara] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    const denied = await createForum(cara, anchor, DENIED);
    deniedId = denied;
    writable = await createForum(cara, anchor, WRITABLE);
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

  it("keeps New post disabled with the Host's reason where posting is denied, and the Host refuses in words", async () => {
    await openForum(DENIED);
    // Never hidden: the control stays, disabled, with the Host's reason (FT1d).
    const newPost = await shown(browser, ".forum-channel-new-post-btn", NET_WAIT_MS);
    await waitUntil(async () => !(await newPost.isEnabled()), NET_WAIT_MS, "New post stayed enabled in the forum where posting is denied");
    const reason = (await newPost.getAttribute("title")) ?? "";
    const described = (await newPost.getAttribute("aria-description")) ?? "";
    console.log(`[forum] New post's reason: ${reason}`);
    // It reads disabled: the theme's disabled text colour (ST1), not the toolbar's text.
    const dimmed = (await browser.execute(() => {
      const btn = document.querySelector(".forum-channel-new-post-btn");
      const probe = document.createElement("span");
      probe.style.color = "var(--color-xfire-text-disabled)";
      document.body.appendChild(probe);
      const disabledColour = getComputedStyle(probe).color;
      probe.remove();
      return !!btn && getComputedStyle(btn).color === disabledColour;
    })) as boolean;
    // Ari is a plain member: "Set topic…" is shown, disabled with the Host's
    // reason, in the disabled text colour, not an active-looking link.
    const topic = (await browser.execute(() => {
      const el = document.querySelector(".community-channel-header .channel-topic");
      const probe = document.createElement("span");
      probe.style.color = "var(--color-xfire-text-disabled)";
      document.body.appendChild(probe);
      const disabledColour = getComputedStyle(probe).color;
      probe.remove();
      return {
        disabled: el?.getAttribute("aria-disabled") === "true",
        reason: (el?.getAttribute("title") ?? "").length > 0,
        dimmed: !!el && getComputedStyle(el).color === disabledColour,
      };
    })) as { disabled: boolean; reason: boolean; dimmed: boolean };
    await shot("new-post-disabled");
    // The Host's own refusal, asked directly as the button would: its reason
    // is in words, never a permission bitmask or a channel id (FT1d).
    const refusal = (await browser.execute(async (community: string, channel: string) => {
      const { invoke } = (window as unknown as {
        __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> };
      }).__TAURI_INTERNALS__;
      try {
        await invoke("create_forum_post", { communityId: community, channelId: channel, title: "Refused", forumTag: null, body: "Refused" });
        return { code: 0, message: "" };
      } catch (e) {
        const failed = e as { type?: string; data?: { code?: number; message?: string } };
        return { code: failed.data?.code ?? -1, message: failed.data?.message ?? JSON.stringify(e) };
      }
    }, anchor, deniedId)) as { code: number; message: string };
    console.log(`[forum] the Host's refusal: ${JSON.stringify(refusal)}`);
    expect({
      disabledWithReason: reason.length > 0 && described === reason,
      dimmed,
      topic,
      // The empty forum states the Host's reason, never the invitation to post.
      emptyState: (await (await shown(browser, ".forum-channel-empty-subtitle")).getText()).trim() === reason,
      refused: refusal.code,
      bitmask: /0x[0-9a-f]{4,}/i.test(refusal.message) || /0x[0-9a-f]{4,}/i.test(reason),
      channelId: refusal.message.includes(deniedId) || reason.includes(deniedId),
      cards: (await browser.$$(".forum-channel-thread-card")).length,
    }).toEqual({ disabledWithReason: true, dimmed: true, topic: { disabled: true, reason: true, dimmed: true }, emptyState: true, refused: 403, bitmask: false, channelId: false, cards: 0 });
  });

  it("shows the accepted starter once, including after opening the thread again", async () => {
    await openForum(WRITABLE);
    await composePost();
    await gone(browser, COMPOSER, NET_WAIT_MS);
    await byText(browser, ".thread-panel-title", TITLE, NET_WAIT_MS);
    await byText(browser, THREAD_MESSAGE, BODY, NET_WAIT_MS);
    const [id] = await starterRows();
    // The Host's id for the starter is opaque to the shell (SEC5: a reply is
    // keyed by its author and id), so only its presence and uniqueness count.
    expect((id ?? "").length).toBeGreaterThan(0);
    expect(await starterRows()).toEqual([id]);
    // The Host counts our own queued starter (FT1d): the card and the open
    // thread read "1 message" before it is sent.
    await waitUntil(
      async () => / · 1 message · /.test(await (await byText(browser, ".forum-channel-thread-card", TITLE)).getText()),
      NET_WAIT_MS,
      "the post's card did not read 1 message before the starter was sent",
    );
    await waitUntil(
      async () => / · 1 message\b/.test(await (await browser.$(".thread-panel-meta")).getText()),
      NET_WAIT_MS,
      "the open thread's header did not read 1 message before the starter was sent",
    );
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
    // The sent starter is the thread's one message, so its card counts it in words.
    await waitUntil(
      async () => / · 1 message · /.test(await (await byText(browser, ".forum-channel-thread-card", TITLE)).getText()),
      NET_WAIT_MS,
      "the post's card never read 1 message",
    );
    // The open thread's header follows the same served row (it kept "0 messages").
    await waitUntil(
      async () => / · 1 message\b/.test(await (await browser.$(".thread-panel-meta")).getText()),
      NET_WAIT_MS,
      "the open thread's header never read 1 message",
    );
    await shot("starter-sent");
    // Ari is the only one in the forum: the channel's presence badge counts the
    // other members here, so it shows nothing (it read "1" with a dot, Ari
    // herself, like an unread count).
    expect(await browser.$$(".channel-presence-badge")).toHaveLength(0);
  });

  it("closes the open thread when the owner takes away our view of its forum", async () => {
    // The thread is still open from the case before.
    await byText(browser, ".thread-panel-title", TITLE, NET_WAIT_MS);
    // Every toast from here, since each shows for a few seconds only.
    await browser.execute(() => {
      const seen: string[] = [];
      (window as unknown as { __toasts: string[] }).__toasts = seen;
      new MutationObserver(() => {
        for (const t of document.querySelectorAll(".toast-item")) {
          const text = (t.textContent ?? "").trim();
          if (text && !seen.includes(text)) seen.push(text);
        }
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    });
    const [cara] = peers;
    await peerReviewed(cara!, [
      "community", "channel", "overwrite", "set", "-c", anchor, "-C", writable,
      "--target", "everyone", "--deny", VIEW_CHANNELS,
    ]);
    const from = Date.now();
    await waitUntil(async () => !(await browser.$(".thread-panel").isExisting()), KEY_WAIT_MS, "").catch(() => {
      throw new Error(`the open thread still shows ${KEY_WAIT_MS / 1000} s after Cara denied everyone the forum's view`);
    });
    console.log(`[forum] the thread closed ${Math.round((Date.now() - from) / 1000)} s after the view deny`);
    // The forum stays listed, shown locked with the Host's reason (its served
    // lock), never as an empty forum; New post is disabled with that reason.
    const locked = await shown(browser, ".forum-channel-locked", NET_WAIT_MS);
    const lockText = (await locked.getText()).trim();
    const newPostAfter = await shown(browser, ".forum-channel-new-post-btn");
    console.log(`[forum] locked forum reads: ${JSON.stringify(lockText)}`);
    const toasts = (await browser.execute(() => (window as unknown as { __toasts?: string[] }).__toasts ?? [])) as string[];
    console.log(`[forum] toasts: ${JSON.stringify(toasts)}`);
    await shot("view-revoked");
    expect({
      closedWithReason: toasts.includes("This thread is no longer available."),
      // The Host's refusal of the thread list is said once, not per re-read.
      refusalToasts: toasts.filter((t) => t.startsWith("thread list")).length <= 1,
      // The Host's refusal is in words (FT1e): never a permission mask or a 32-hex id.
      rawRefusal: toasts.filter((t) => /0x[0-9a-f]+/i.test(t) || /\b[0-9a-f]{32}\b/i.test(t)),
      cards: (await browser.$$(".forum-channel-thread-card")).length,
      lockedWithReason: lockText.replace("This forum is locked", "").trim().length > 0,
      newPostDisabled: !(await newPostAfter.isEnabled()),
    }).toEqual({ closedWithReason: true, refusalToasts: true, rawRefusal: [], cards: 0, lockedWithReason: true, newPostDisabled: true });
  });
});
