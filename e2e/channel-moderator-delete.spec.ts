import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// A moderator's delete (plan E3.3 M4, E4): the window shows the Host's
// `Deleted` event naming the moderator as "Removed by <name>", and the
// Host's review of a bulk delete says the messages are "deleted for
// members", never erased. The samples are the golden JSON `rekindle-types`
// serializes from the Rust types.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden samples' community
const CHANNEL = `ch_${"c".repeat(32)}`; // the golden delete's channel
const MESSAGE = `msg_${"1".repeat(32)}`; // the golden delete's message
const ME = "e".repeat(64);
const CAL = "c".repeat(64); // the golden delete's moderator
const EVERYONE = "00000000000000000000000000000000";
// VIEW_CHANNELS | SEND_MESSAGES as the window reads them, plus MANAGE_MESSAGES (1 << 21).
const PERMISSIONS = (65536n | (1n << 21n)).toString();

const CAL_MEMBER = {
  pseudonymKey: CAL, displayName: "Cal", roleIds: [EVERYONE], displayRole: "@everyone",
  status: "online", timeoutUntil: null, lastActive: 0, group: { kind: "online" },
};

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Bea" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [{ id: "${CHANNEL}", name: "general", channelType: "text" }],
        roles: [{ id: "${EVERYONE}", name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: ["${EVERYONE}"], myPermissions: "${PERMISSIONS}",
        myChannelPermissions: {}, removed: null,
      }];
    case "get_community_members":
      return [${JSON.stringify(CAL_MEMBER)}];
    case "get_channel_messages":
      return [{ id: 1, senderId: "${"d".repeat(64)}", body: "spam from dee", timestamp: 1760000000000,
                isOwn: false, serverMessageId: "${MESSAGE}", sendState: "sent" }];
    case "bulk_delete_channel_messages":
      throw {
        type: "intentRequired",
        data: {
          intentId: "01900000-0000-7000-8000-000000000001",
          kind: "bulkDeleteMessages", class: "destructive", subject: { named: { name: "general" } },
          facts: [${JSON.stringify(GOLDEN["intent.fact.messagesDeleted"])}], escalations: [],
          reversibility: "never", notifies: "everyone", proof: "commit",
          expiresAtMs: Date.now() + 60000, cancelSends: false,
        },
      };
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

async function open(page: Page): Promise<void> {
  await preloadMocks(page, "community", HANDLER);
  await openWindow(page, `/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
  await expect(page.getByText("spam from dee", { exact: true })).toBeVisible();
}

test("a moderator's delete shows Removed by the moderator's name", async ({ page }) => {
  await open(page);
  await page.evaluate((payload) => {
    const w = window as any;
    w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
      index: 0,
      message: { cursor: null, channel: "community-event", payload },
    });
  }, GOLDEN["channelMessage.deleted"]);
  await expect(page.locator(".message-removed")).toHaveText("Removed by Cal");
  await expect(page.getByText("spam from dee", { exact: true })).toHaveCount(0);
});

test("the review of a bulk delete says deleted for members", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Select messages…" }).click();
  await page.getByRole("checkbox").first().check();
  await page.locator(".message-list-selection-btn-danger").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("1204 messages are deleted for members.")).toBeVisible();
  await expect(dialog.getByText(/erased|for everyone/)).toHaveCount(0);
});
