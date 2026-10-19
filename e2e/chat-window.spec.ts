import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// The one chat window (plan E2.3): `?peer=` is the 1:1 mode, `?group=` the
// group mode, and typing is decided in the backend — the window reports
// its draft and renders the typing events it is sent.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const ALICE = "a".repeat(64);
// The golden typing samples name this peer.
const BOB = "b".repeat(64);
const GROUP = `VLD0:${"G".repeat(43)}`;

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
        lastSeenAt: null, friendshipState: "accepted", handshakeError: null,
      }];
    case "list_dms":
      return [{
        recordKey: "${GROUP}", isGroup: true, initiatorPublicKey: "${BOB}",
        initiatorPseudonym: "Carol", mySubkey: 1, participants: [], mekGeneration: 0,
        createdAt: 1, lastMessageAt: null,
      }];
    case "get_dm_messages":
      return [{
        id: 1, senderPseudonym: "Carol", body: "hello group", timestamp: 1,
        sequence: 1, mekGeneration: 0,
      }];
    case "get_message_history":
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

/** The `send_typing` reports the window made, in order. */
async function typingReports(page: Page): Promise<boolean[]> {
  return page.evaluate(() =>
    ((window as any).__ipcCalls as { cmd: string; args: { typing: boolean } }[])
      .filter((c) => c.cmd === "send_typing")
      .map((c) => c.args.typing),
  );
}

test.beforeEach(() => {
  nextIndex = 0;
});

test("the 1:1 mode renders the typing the backend emits", async ({ page }) => {
  await preloadMocks(page, `chat-${BOB.slice(0, 16)}`, HANDLER);
  await page.goto(`/chat?peer=${BOB}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.locator(".chat-call-btn").first()).toBeVisible();

  await deliver(page, "typing.started.dm", "chat-event");
  await expect(page.getByText("Bob is typing...")).toBeVisible();
  await deliver(page, "typing.stopped.dm", "chat-event");
  await expect(page.getByText("Bob is typing...")).toHaveCount(0);
});

test("the 1:1 mode reports its draft and keeps no typing timer", async ({ page }) => {
  await preloadMocks(page, `chat-${BOB.slice(0, 16)}`, HANDLER);
  await page.goto(`/chat?peer=${BOB}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");

  const input = page.locator("textarea.message-input-field");
  await input.fill("h");
  await input.fill("hi");
  await input.fill("");
  await expect.poll(() => typingReports(page)).toEqual([true, true, false]);
  // No stop of its own after the old 3 s idle timer: the backend owns it.
  await page.waitForTimeout(3500);
  expect(await typingReports(page)).toEqual([true, true, false]);
});

test("the group mode renders the group's rows and no call controls", async ({ page }) => {
  await preloadMocks(page, "chat-VLD0GGGGGGGGGGGGGGGG", HANDLER);
  await page.goto(`/chat?group=${encodeURIComponent(GROUP)}`);
  await expect(page.getByText("hello group")).toBeVisible();
  await expect(page.getByText("Chat — Carol (group)")).toBeVisible();
  await expect(page.locator(".chat-call-btn")).toHaveCount(0);
});
