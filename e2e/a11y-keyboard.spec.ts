import { test, expect, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Plan E7.3 (built with E7.2's slice 2): toast live regions, and the buddy,
// channel and member lists as roving-tabindex lists (wiring guide §13f).

const ME = "a".repeat(64);
const COMMUNITY = `VLD0:${"g".repeat(43)}`;

function friend(key: string, name: string): string {
  return `{
    publicKey: "${key}", displayName: "${name}", nickname: null, status: "online",
    statusMessage: null, gameInfo: null, groupId: null, group: null,
    groupLabel: "Friends", unreadCount: 0, lastSeenAt: null,
    friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
    identityPinMismatch: false,
  }`;
}

const BUDDIES = `
  switch (cmd) {
    case "get_identity":
      return { publicKey: "${ME}", displayName: "Me" };
    case "get_friends":
      return [${friend("b".repeat(64), "Bob")}, ${friend("c".repeat(64), "Cy")}, ${friend("d".repeat(64), "Di")}];
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

const CHANNELS = `
  switch (cmd) {
    case "get_identity":
      return { publicKey: "${ME}", displayName: "Me" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [
          { id: "ch_${"1".repeat(32)}", name: "general", channelType: "text" },
          { id: "ch_${"2".repeat(32)}", name: "maps", channelType: "text" },
        ],
        roles: [{ id: "00000000000000000000000000000000", name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${"e".repeat(64)}", myRoleIds: ["00000000000000000000000000000000"], myPermissions: "65536",
        myChannelPermissions: {}, removed: null,
      }];
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

function calls(page: Page, cmd: string): Promise<unknown[]> {
  return page.evaluate(
    (c) => ((window as any).__ipcCalls ?? []).filter((x: { cmd: string }) => x.cmd === c).map((x: { args: unknown }) => x.args),
    cmd,
  );
}

async function openBuddies(page: Page): Promise<void> {
  await preloadMocks(page, "buddy-list", BUDDIES);
  await page.goto("/buddy-list");
  await expect(page.locator(".buddy-list .buddy-item")).toHaveCount(3, { timeout: 10_000 });
}

test("errors are alerts, other toasts status messages, and both regions exist from mount", async ({ page }) => {
  await page.clock.install();
  await openBuddies(page);
  const status = page.locator(".toast-container [role=status]");
  const alert = page.locator(".toast-container [role=alert]");
  await expect(status).toHaveCount(1);
  await expect(alert).toHaveCount(1);
  // The module is the dev server's URL, resolved in the page, so it is passed
  // in as data; its type is the source module's.
  await page.evaluate(async (url) => {
    const t: typeof import("../src/stores/toast.store") = await import(url);
    t.addToast("Saved", "success");
    t.addToast("Not sent", "error");
    t.addToast("Done.", "info", { label: "Undo", run: () => undefined });
  }, "/src/stores/toast.store.ts");
  await expect(status.locator(".toast-item")).toHaveText(["Saved", "Done.Undo"]);
  await expect(alert.locator(".toast-item")).toHaveText(["Not sent"]);
  // SC 2.2.1: a toast with an action outlives the 4 s timer the others have.
  await page.clock.runFor(5_000);
  await expect(page.locator(".toast-item")).toHaveText(["Done.Undo"]);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.locator(".toast-item")).toHaveCount(0);
});

test("the buddy list is one tab stop, walked with the arrows, opened with Enter", async ({ page }) => {
  await openBuddies(page);
  const rows = page.locator(".buddy-list .buddy-item");
  await expect(page.locator(".buddy-list .buddy-item[tabindex=\"0\"]")).toHaveCount(1);
  await rows.first().focus();
  await page.keyboard.press("ArrowDown");
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press("End");
  await expect(rows.nth(2)).toBeFocused();
  await page.keyboard.press("Home");
  await expect(rows.nth(0)).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.locator(".buddy-list .buddy-item[tabindex=\"0\"]")).toHaveCount(1);
  await expect(rows.nth(1)).toHaveAttribute("tabindex", "0");
  await expect(rows.nth(1)).toHaveClass(/buddy-item-selected/);
  await page.keyboard.press("Enter");
  await expect.poll(() => calls(page, "open_chat_window")).toHaveLength(1);
});

test("Shift+F10 and the Menu key open a buddy's context menu", async ({ page }) => {
  await openBuddies(page);
  await page.locator(".buddy-list .buddy-item").first().focus();
  await page.keyboard.press("Shift+F10");
  await expect(page.locator(".context-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".context-menu")).toHaveCount(0);
  await page.locator(".buddy-list .buddy-item").nth(1).focus();
  await page.keyboard.press("ContextMenu");
  await expect(page.locator(".context-menu")).toBeVisible();
});

test("the channel list is a roving list: arrows move, Enter opens", async ({ page }) => {
  await preloadMocks(page, "community", CHANNELS);
  await page.goto(`/community?id=${COMMUNITY}`);
  const rows = page.locator(".channel-item");
  await expect(rows).toHaveCount(2, { timeout: 10_000 });
  await expect(page.locator('.channel-item[tabindex="0"]')).toHaveCount(1);
  await rows.first().focus();
  await page.keyboard.press("ArrowDown");
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(rows.nth(1)).toHaveClass(/channel-item-selected/);
});
