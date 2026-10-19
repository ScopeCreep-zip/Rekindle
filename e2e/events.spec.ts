import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// Live events reach the real handlers in the shape Rust emits (plan C7.26).
//
// The events are the golden samples `rekindle-types` serializes from the
// Rust `SubscriptionEvent` types (`crates/rekindle-types/tests/golden/
// subscription_events.json`), not hand-written payloads: before C7.26 the
// inner families went out as `{"friend":{"RequestReceived":{"from_key":…}}}`
// while the handlers read `requestReceived`/`fromKey`, and a hand-written
// camelCase fixture could never have caught it.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

// The buddy list hydrates Bob as a friend we sent a request to; the mock
// keeps the window's event channel id so the test can deliver on it.
const BUDDY_LIST_HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${ALICE}", displayName: "Alice" };
    case "get_friends":
      return [{
        publicKey: "${BOB}", displayName: "Bob", nickname: null, status: "offline",
        statusMessage: null, gameInfo: null, group: null, unreadCount: 0,
        lastSeenAt: null, friendshipState: "pendingOut", handshakeError: null,
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

/** Deliver one golden sample on the window's event channel, as the backend does. */
async function deliver(page: Page, sample: string, channel: string): Promise<void> {
  const payload = GOLDEN[sample];
  expect(payload, `golden sample ${sample}`).toBeDefined();
  const index = nextIndex++;
  await page.evaluate(
    ({ index, envelope }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, { index, message: envelope });
    },
    { index, envelope: { cursor: null, channel, payload } },
  );
}

test.beforeEach(async ({ page }) => {
  nextIndex = 0;
  await preloadMocks(page, "buddy-list", BUDDY_LIST_HANDLER);
  await page.goto("/buddy-list");
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.getByText("Bob", { exact: true })).toBeVisible();
});

test("a failed first-contact handshake shows on the friend row, and accept clears it", async ({
  page,
}) => {
  await deliver(page, "friend.handshakeFailed", "chat-event");
  await expect(page.locator(".buddy-handshake-error")).toHaveText("Secure session not set up");

  await deliver(page, "friend.accepted", "chat-event");
  await expect(page.locator(".buddy-handshake-error")).toHaveCount(0);
});

test("an incoming friend request lists under pending requests", async ({ page }) => {
  await deliver(page, "friend.requestReceived", "chat-event");
  await expect(page.getByText("Pending Requests (1)")).toBeVisible();
});

test("a direct message raises the friend's unread count", async ({ page }) => {
  await deliver(page, "channelMessage.directMessageReceived", "chat-event");
  await expect(page.locator(".buddy-unread-badge")).toHaveText("1");
});
