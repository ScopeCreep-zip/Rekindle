import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Walk 18 (d): a created channel is listed once. The Host merges our own
// ChannelCreated and publishes `channelsChanged` (the whole list, the new
// channel in it) before the create call answers; the create's answer
// must not add the channel a second time.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const GENERAL = `ch_${"c".repeat(32)}`;
const NEW = `ch_${"n".repeat(32)}`;
const ME = "e".repeat(64);

const overview = (id: string, name: string, kind: string) => ({
  id, name, kind, categoryId: null, topic: "", mekGeneration: 0, logKey: null, sortOrder: 0,
  slowmodeSeconds: null,
});

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Ann" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [{ id: "${GENERAL}", name: "general", channelType: "text" }],
        roles: [{ id: 0, name: "@everyone", color: 0, permissions: "${(1n << 50n).toString()}",
                  position: 0, hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: [0], removed: null,
      }];
    case "create_channel":
      window.__createAsked = true;
      return new Promise((resolve) => {
        window.__answerCreate = () => resolve({ id: "${NEW}" });
      });
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

test("a channel the Host announced before the create answered is listed once", async ({ page }) => {
  await preloadMocks(page, "community", HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.locator('[title="Create Channel"]').click();
  await page.locator('input[placeholder="Channel name..."]').fill("walk-voice");
  await page.locator(".modal-container select.form-select").selectOption("voice");
  await page.locator('.modal-container button[type="submit"]').click();
  await page.waitForFunction(() => (window as any).__createAsked === true);
  const payload = {
    governance: {
      channelsChanged: {
        community: COMMUNITY,
        channels: [overview(GENERAL, "general", "text"), overview(NEW, "walk-voice", "voice")],
        categories: [],
      },
    },
  };
  await page.evaluate((payload) => {
    const w = window as any;
    w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
      index: 0,
      message: { cursor: null, channel: "community-event", payload },
    });
  }, payload);
  await expect(page.locator(".channel-list")).toContainText("walk-voice");
  await page.evaluate(() => (window as any).__answerCreate());
  await expect(page.locator('input[placeholder="Channel name..."]')).toHaveCount(0);
  await expect(page.locator(".channel-list").getByText("walk-voice", { exact: true })).toHaveCount(1);
});
