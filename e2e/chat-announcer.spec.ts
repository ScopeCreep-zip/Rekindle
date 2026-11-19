import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.3, wiring guide §13b (WCAG 2.2 SC 4.1.3): the chat window
// announces a message the peer sends while it is open, politely and
// without moving focus. Stored history and our own sends are not read out.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, any>;

const ALICE = "a".repeat(64);
// The golden DM and group samples name this peer and this group.
const BOB = "b".repeat(64);
const GROUP = "6d".repeat(16);

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${ALICE}", displayName: "Alice" };
    case "get_friends":
      return [{
        publicKey: "${BOB}", displayName: "Bob", nickname: null, status: "online",
        statusMessage: null, gameInfo: null, group: null, unreadCount: 0,
        lastSeenAt: null, friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
      }];
    case "get_message_history":
    case "group_dm_messages":
      return [{
        id: 1, messageId: null, senderId: "${BOB}", body: "stored line", timestamp: 1700000000000,
        isOwn: false, sendState: "sent", historyGapThrough: null, expireSeconds: 0, expireAt: null,
      }];
    case "group_dm_list":
      return [{
        groupId: "${GROUP}", name: "raid night", epoch: 0, pending: false,
        members: [{ key: "${ALICE}", label: "Alice" }, { key: "${BOB}", label: "Bob" }],
      }];
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

let nextIndex = 0;

/** Deliver one event on the window's channel, as the backend does. */
async function deliver(page: Page, payload: unknown): Promise<void> {
  const index = nextIndex++;
  await page.evaluate(
    ({ index, envelope }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, { index, message: envelope });
    },
    { index, envelope: { cursor: null, channel: "chat-event", payload } },
  );
}

function dm(isOwn: boolean, body: string, serverMessageId: string): unknown {
  const sample = GOLDEN["channelMessage.directMessageReceived"];
  expect(sample).toBeDefined();
  return {
    channelMessage: {
      directMessageReceived: { ...sample.channelMessage.directMessageReceived, isOwn, body, serverMessageId, automodBlurred: false },
    },
  };
}

test.beforeEach(() => {
  nextIndex = 0;
});

test("the 1:1 chat announces the peer's new message, not history or our own", async ({ page }) => {
  await preloadMocks(page, `chat-${BOB.slice(0, 16)}`, HANDLER);
  await openWindow(page, `/chat?peer=${BOB}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.locator(".chat-message", { hasText: "stored line" })).toBeVisible();

  const announcer = page.locator(".chat-announcer");
  await expect(announcer).toHaveAttribute("aria-live", "polite");
  await expect(announcer).toHaveAttribute("aria-relevant", "additions");
  // The list itself is silent, so loading history is never read out.
  await expect(page.locator(".chat-message-area")).toHaveAttribute("aria-live", "off");
  await expect(announcer).toHaveText("");

  const input = page.getByRole("combobox", { name: "Message Bob" });
  await input.focus();
  await deliver(page, dm(true, "sent from my laptop", "01900000-0000-7000-8000-0000000000a1"));
  await expect(page.locator(".chat-message", { hasText: "sent from my laptop" })).toBeVisible();
  await deliver(page, dm(false, "gg wp", "01900000-0000-7000-8000-0000000000a2"));
  await expect(page.locator(".chat-message", { hasText: "gg wp" })).toBeVisible();

  await expect(announcer.locator("p")).toHaveCount(1);
  await expect(announcer.locator("p")).toHaveText(/^Bob at .+: gg wp$/);
  await expect(input).toBeFocused();
});

test("the group chat announces a member's new message", async ({ page }) => {
  await preloadMocks(page, `chat-${GROUP.slice(0, 16)}`, HANDLER);
  await openWindow(page, `/chat?group=${GROUP}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.locator(".chat-message", { hasText: "stored line" })).toBeVisible();

  const announcer = page.locator(".chat-announcer");
  await expect(announcer).toHaveText("");
  const sample = GOLDEN["groupDm.message"];
  expect(sample).toBeDefined();
  await deliver(page, sample);
  await expect(page.locator(".chat-message", { hasText: "hello group" })).toBeVisible();
  await expect(announcer.locator("p")).toHaveText([/^Bob at .+: hello group$/]);
});
