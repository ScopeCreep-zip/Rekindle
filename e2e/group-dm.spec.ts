import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// Group DMs in the buddy list (plan E2.6): "New group" goes through the
// Host's review and lists the created group; an invite arrives as the
// golden `groupDm.invited` event and Accept is reviewed the same way.

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
const CAROL = "c".repeat(64);
const NEW_GROUP = "0123456789abcdef0123456789abcdef";
const INVITED = "6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d";

const friend = (key: string, name: string) => `{
  publicKey: "${key}", displayName: "${name}", nickname: null, status: "offline",
  statusMessage: null, gameInfo: null, group: null, unreadCount: 0, lastSeenAt: null,
  friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
}`;

const review = (kind: string, cls: string, name: string) => `{
  type: "intentRequired",
  data: {
    intentId: "01900000-0000-7000-8000-00000000000${kind.length % 10}",
    kind: "${kind}", class: "${cls}", subject: { named: { name: "${name}" } },
    facts: [], escalations: [], reversibility: "never", notifies: "nobody",
    proof: "commit", expiresAtMs: Date.now() + 60000, cancelSends: false,
  },
}`;

// The Host's list answers what the mock committed: the created group, or
// the accepted invite as a member group.
const HANDLER = `
  window.__groups ??= [];
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${ALICE}", displayName: "Alice" };
    case "get_friends":
      return [${friend(BOB, "Bob")}, ${friend(CAROL, "Carol")}];
    case "group_dm_create":
      window.__pending = { name: args.name, members: args.members };
      throw ${review("createGroupDm", "consequential", "raid night")};
    case "group_dm_accept":
      window.__accepting = args.groupId;
      throw ${review("acceptDmInvite", "trustChanging", "raid night")};
    case "intent_commit":
      if (window.__pending) {
        window.__groups.push({
          groupId: "${NEW_GROUP}", name: window.__pending.name, epoch: 0, pending: false,
          members: [{ key: "${ALICE}", label: "Alice" }, { key: "${BOB}", label: "Bob" },
                    { key: "${CAROL}", label: "Carol" }],
        });
        window.__pending = null;
        return "${NEW_GROUP}";
      }
      if (window.__accepting) {
        window.__groups.push({
          groupId: window.__accepting, name: "raid night", epoch: 0, pending: false,
          members: [{ key: "${ALICE}", label: "Alice" }, { key: "${BOB}", label: "Bob" }],
        });
        window.__accepting = null;
      }
      return null;
    case "group_dm_list":
      return window.__groups;
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

async function deliver(page: Page, sample: string): Promise<void> {
  const payload = GOLDEN[sample];
  expect(payload, `golden sample ${sample}`).toBeDefined();
  await page.evaluate(
    ({ envelope }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, { index: 0, message: envelope });
    },
    { envelope: { cursor: null, channel: "chat-event", payload } },
  );
}

test.beforeEach(async ({ page }) => {
  await preloadMocks(page, "buddy-list", HANDLER);
  await page.goto("/buddy-list");
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.getByText("Bob", { exact: true })).toBeVisible();
});

test("New group goes through the Host's review and lists the group", async ({ page }) => {
  await page.getByRole("button", { name: "New Group" }).click();
  const create = page.getByRole("button", { name: "Create", exact: true });
  await page.getByLabel("Group name").fill("raid night");
  await page.getByRole("checkbox", { name: "Bob" }).check();
  // One friend is below the 2-7 range the dialog states.
  await expect(create).toBeDisabled();
  await page.getByRole("checkbox", { name: "Carol" }).check();
  await create.click();

  await page.getByRole("button", { name: /^Create group dm/ }).click();
  await expect(page.getByRole("button", { name: /Open group chat raid night, 3 members/ })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__ipcCalls);
  const sent = calls.find((c: { cmd: string }) => c.cmd === "group_dm_create");
  expect(sent.args).toEqual({ name: "raid night", members: [BOB, CAROL] });
});

test("an invite shows Accept and Decline, and Accept is reviewed", async ({ page }) => {
  await deliver(page, "groupDm.invited");
  await expect(page.getByRole("group", { name: "Invite to raid night" })).toBeVisible();
  await expect(page.getByText("Alice, Bob, cccc-cccc")).toBeVisible();

  await page.getByRole("button", { name: "Accept invite to raid night" }).click();
  await page.getByRole("button", { name: /^Accept dm invite/ }).click();
  await expect(page.getByRole("group", { name: "Invite to raid night" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Open group chat raid night/ })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__ipcCalls);
  expect(calls.some((c: { cmd: string; args: { groupId?: string } }) =>
    c.cmd === "group_dm_accept" && c.args.groupId === INVITED)).toBe(true);
});
