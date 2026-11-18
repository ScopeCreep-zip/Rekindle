import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// An edit of a channel message shows in the window from the Host's `Edited`
// event (plan E7.13, walk row M45), whose sample is the golden JSON
// `rekindle-types` serializes from the Rust type.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden samples' community
const CHANNEL = `ch_${"c".repeat(32)}`; // the golden edit's channel
const MESSAGE = `msg_${"1".repeat(32)}`; // the golden edit's message
const ME = "e".repeat(64);

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
        roles: [{ id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: [0], removed: null,
      }];
    case "get_channel_messages":
      return [{ id: 1, senderId: "${ME}", body: "hello channel", timestamp: 1760000000000,
                isOwn: true, serverMessageId: "${MESSAGE}", sendState: "sent" }];
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
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
  await expect(page.getByText("hello channel", { exact: true })).toBeVisible();
}

async function send(page: Page, cursor: unknown, payload: unknown): Promise<void> {
  await page.evaluate(
    ({ cursor, payload }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: 0,
        message: { cursor, channel: "community-event", payload },
      });
    },
    { cursor, payload },
  );
}

test("the Host's Edited event shows the new body", async ({ page }) => {
  await open(page);
  await send(page, null, GOLDEN["channelMessage.edited"]);
  await expect(page.getByText("hello channel, edited", { exact: true })).toBeVisible();
});
