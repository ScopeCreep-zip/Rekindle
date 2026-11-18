import { test, expect, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// The open thread (plan E7.13, walk row M76): who started it, and what a
// reply shows.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const CHANNEL = `ch_${"c".repeat(32)}`;
const THREAD_ID = `th_${"t".repeat(32)}`;
const ME = "e".repeat(64);
const PEER = "d".repeat(64);

const member = (key: string, name: string) => ({
  pseudonymKey: key,
  displayName: name,
  roleIds: [0],
  displayRole: "@everyone",
  status: "online",
  timeoutUntil: null,
  lastActive: 0,
  group: { kind: "online" },
});

const THREAD = {
  id: THREAD_ID, channelId: CHANNEL, name: "walk thread", starterMessageId: "msg_1",
  creatorPseudonym: PEER, forumTag: null, createdAt: 1760000000, archived: false,
  autoArchiveSeconds: 86400, lastMessageAt: 0, messageCount: 0,
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
        roles: [{ id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: [0], removed: null,
      }];
    case "get_community_members":
      return ${JSON.stringify([member(ME, "Bea"), member(PEER, "Cid")])};
    case "get_active_threads":
      return [${JSON.stringify(THREAD)}];
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

async function openThread(page: Page, handler = HANDLER): Promise<void> {
  await preloadMocks(page, "community", handler);
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
  await page.locator('button[title="Threads"]').click();
  await page.locator(".thread-list-item", { hasText: "walk thread" }).click();
  await expect(page.locator(".thread-panel-title")).toHaveText("walk thread");
}

test("the thread header names its starter, not the pseudonym", async ({ page }) => {
  await openThread(page);
  const meta = page.locator(".thread-panel-meta");
  await expect(meta).toContainText("Started by Cid");
  await expect(meta).not.toContainText(PEER);
});
