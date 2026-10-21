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
const GROUP = "0123456789abcdef0123456789abcdef";

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

// The group mode reads the Host's list and stored history; a group that
// is not in our list cannot be sent to, and the mode has no calls.
test("the group mode shows the group's members and history, and no call controls", async ({ page }) => {
  await preloadMocks(page, `chat-g${GROUP}`, HANDLER);
  await page.goto(`/chat?group=${GROUP}`);
  await expect(page.getByText("Chat — Group DM")).toBeVisible();
  await expect(page.getByText("You can send once you are in this group.")).toBeVisible();
  await expect(page.locator(".chat-call-btn")).toHaveCount(0);
});

const CLIENT = "0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5a";

// A group we are in: the Host commits our message with its queued echo
// (group_dm_send, then the stored history), and the echo's state follows
// the Host's sendStatus.
const GROUP_HANDLER = `
  window.__rows ??= [];
  switch (cmd) {
    case "group_dm_list":
      return [{ groupId: "${GROUP}", name: "raid night", epoch: 0, pending: false,
        members: [{ key: "${ALICE}", label: "Alice" }, { key: "${BOB}", label: "Bob" }] }];
    case "group_dm_messages":
      return window.__rows;
    case "group_dm_send":
      window.__rows.push({ id: 1, messageId: "${CLIENT}", senderId: "${ALICE}", body: args.body,
        timestamp: Date.now(), isOwn: true, sendState: "queued", historyGapThrough: null });
      return "${CLIENT}";
  }
  ${HANDLER}
`;

test("a group message is sent queued, then sent as the Host reports it", async ({ page }) => {
  await preloadMocks(page, `chat-g${GROUP}`, GROUP_HANDLER);
  await page.goto(`/chat?group=${GROUP}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.getByText("Chat — raid night")).toBeVisible();
  const input = page.locator("textarea.message-input-field");
  await input.fill("gg all");
  await input.press("Enter");
  await expect(page.getByText("gg all")).toBeVisible();
  await expect(page.locator(".message-status-queued")).toBeVisible();

  const golden = GOLDEN["groupDm.sendStatus"] as { groupDm: { sendStatus: object } };
  const payload = { groupDm: { sendStatus: { ...golden.groupDm.sendStatus, groupId: GROUP, clientId: CLIENT, state: "sent" } } };
  const index = nextIndex++;
  await page.evaluate(
    ({ index, envelope }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, { index, message: envelope });
    },
    { index, envelope: { cursor: null, channel: "chat-event", payload } },
  );
  await expect(page.locator(".message-status-sent")).toBeVisible();
  await expect(page.locator(".message-status-queued")).toHaveCount(0);

  const calls = await page.evaluate(() => (window as any).__ipcCalls);
  const sent = calls.find((c: { cmd: string }) => c.cmd === "group_dm_send");
  expect(sent.args).toMatchObject({ groupId: GROUP, body: "gg all" });
  const typing = calls.filter((c: { cmd: string }) => c.cmd === "group_dm_typing");
  expect(typing[0].args).toMatchObject({ groupId: GROUP, typing: true });
});
