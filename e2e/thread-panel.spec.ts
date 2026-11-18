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
    case "get_channel_threads":
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

async function sendEvent(page: Page, index: number, payload: unknown): Promise<void> {
  await page.evaluate(({ index, payload }) => {
    const w = window as any;
    w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
      index,
      message: { cursor: null, channel: "community-event", payload },
    });
  }, { index, payload });
}

const posted = (messageId: string, sender: string, body: string) => ({
  social: {
    threadMessagePosted: {
      community: COMMUNITY, threadId: THREAD_ID, messageId, senderPseudonym: sender,
      timestamp: 1760000100, body, replyToId: null,
    },
  },
});

// Walk 14-15, M76: a reply sent while the thread's first page is still
// loading must survive that page's late answer.
test("a reply sent before the thread's page loads still shows", async ({ page }) => {
  const slowPage = HANDLER.replace(
    'case "get_welcome_screen":',
    `case "get_thread_messages":
      return new Promise((resolve) => { window.__releasePage = () => resolve([]); });
    case "get_welcome_screen":`,
  );
  await openThread(page, slowPage);
  const input = page.locator(".thread-panel .message-input-field");
  await input.fill("thread reply");
  await input.press("Enter");
  await expect(page.locator(".thread-panel-messages")).toContainText("thread reply");
  await page.waitForFunction(() => typeof (window as any).__releasePage === "function");
  await page.evaluate(() => (window as any).__releasePage());
  await page.waitForTimeout(200);
  await expect(page.locator(".thread-panel-messages")).toContainText("thread reply");
});

// M76: the Host publishes our own reply (the mesh never echoes it); the
// window shows it even when no optimistic bubble is waiting for it.
test("our own posted reply shows without a waiting bubble", async ({ page }) => {
  await openThread(page);
  await sendEvent(page, 0, posted("tm_1", ME, "sent from another window"));
  await expect(page.locator(".thread-panel-messages")).toContainText("sent from another window");
});

// M76: a posted reply counts on the open thread's header.
test("a posted reply counts in the thread header", async ({ page }) => {
  await openThread(page);
  await expect(page.locator(".thread-panel-meta")).toContainText("0 messages");
  await sendEvent(page, 0, posted("tm_2", PEER, "count me in"));
  await expect(page.locator(".thread-panel-messages")).toContainText("count me in");
  await expect(page.locator(".thread-panel-meta")).toContainText("1 messages");
});

const archiveChanged = (archived: boolean) => ({
  social: { threadArchiveChanged: { community: COMMUNITY, threadId: THREAD_ID, archived } },
});

// Walk 18 (a)/(b): an archived thread stays reachable. The Host publishes
// the archive to every window; the open thread shows it archived with an
// Unarchive action, the list shows it apart, and the badge counts only
// active threads.
test("an archived thread shows archived and offers Unarchive", async ({ page }) => {
  await openThread(page, HANDLER.replace(
    'case "get_welcome_screen":',
    `case "unarchive_thread":
      window.__unarchived = args.threadId === "${THREAD_ID}";
      return null;
    case "get_welcome_screen":`,
  ));
  await expect(page.locator(".thread-panel-archived")).toHaveCount(0);
  await sendEvent(page, 0, archiveChanged(true));
  const banner = page.locator(".thread-panel-archived");
  await expect(banner).toContainText("This thread is archived");
  await banner.getByRole("button", { name: "Unarchive" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__unarchived)).toBe(true);
  await sendEvent(page, 1, archiveChanged(false));
  await expect(page.locator(".thread-panel-archived")).toHaveCount(0);
});

test("the thread list shows an archived thread apart and the badge leaves", async ({ page }) => {
  // The Host stores the archive before it publishes it, so a re-read
  // after the event lists the thread archived.
  await openThread(page, HANDLER.replace(
    'case "get_channel_threads":',
    `case "get_channel_threads":
      if (window.__archived) return [{ ...${JSON.stringify(THREAD)}, archived: true }];`,
  ));
  const badge = page.locator('button[title="Threads"] .channel-header-badge');
  await expect(badge).toHaveText("1");
  await page.evaluate(() => { (window as any).__archived = true; });
  await sendEvent(page, 0, archiveChanged(true));
  await expect(badge).toHaveCount(0);
  await page.locator(".thread-panel .modal-close-btn").click();
  await page.locator('button[title="Threads"]').click();
  await expect(page.locator(".thread-list-section")).toHaveText("Archived threads");
  await expect(page.locator(".thread-list-item.thread-archived")).toContainText("walk thread");
});
