import { test, expect, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.13 walk finding W6: the sidebar's links are the only way to the
// events panel and the game-server list (with its Add form). They were drawn
// only once an upcoming event existed, or more than three servers, so a new
// community could never create its first event or add its first server.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const ME = "e".repeat(64);
// MANAGE_COMMUNITY (1 << 3) | SEND_MESSAGES (1 << 16), as decimal strings.
const PERMS = String((1n << 3n) | (1n << 16n));

const handler = (welcome: string): string => `
  switch (cmd) {
    case "subscribe_events":
      return null;
    // The Host's empty default (OnboardingAdapter::welcome_screen), never
    // the catch-all's []: handleLoadWelcomeScreen took [] for a set screen
    // and WelcomeScreen read [].channels.length inside that store write.
    // Solid's runUpdates drops its queued effects when a computation throws
    // (Effects = null), so the main pane's insert effect stayed PENDING and
    // no later Show change reached .community-main (walk W6b).
    case "get_welcome_screen":
      return ${welcome};
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Bea" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [{ id: "ch1", name: "general", channelType: "text" }],
        roles: [{ id: "00000000000000000000000000000000", name: "@everyone", color: 0, permissions: "${PERMS}", position: 0,
          hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: ["00000000000000000000000000000000"], myPermissions: "${PERMS}", myChannelPermissions: {}, removed: null,
      }];
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

async function openCommunity(page: Page, welcome: string): Promise<void> {
  await preloadMocks(page, "community", handler(welcome));
  await openWindow(page, `/community?id=${COMMUNITY}`);
  // The window selects the community (closing both panels) once hydrated;
  // the channel header is drawn only after that selection.
  await expect(page.locator(".community-channel-header-name")).toHaveText("general");
}

test.describe("a well-formed welcome screen", () => {
  test.beforeEach(async ({ page }) => openCommunity(page, `{ description: "", channels: [] }`));

  test("with no events, All events opens the events panel from an open channel", async ({ page }) => {
    // Walk W6b: with the mock's [] welcome screen the panel ran without being
    // inserted (see get_welcome_screen above).
    const link = page.locator(".sidebar-events-section .sidebar-section-more", { hasText: "All events" });
    await expect(link).toBeVisible();
    await link.click();
    await expect(page.locator(".community-main .event-list")).toContainText("Community Events");
    await expect(page.locator(".community-main .event-empty")).toHaveText("No events");
    await expect(page.locator(".community-channel-header-name")).toHaveCount(0);
    // A channel click leaves the panel for that channel.
    await page.locator(".channel-item", { hasText: "general" }).click();
    await expect(page.locator(".community-channel-header-name")).toHaveText("general");
    await expect(page.locator(".community-main .event-list")).toHaveCount(0);
  });

  test("with no game servers, All servers opens the list and its Add form", async ({ page }) => {
    await expect(page.locator(".sidebar-servers-section")).toContainText("Servers (0)");
    await page.locator(".sidebar-servers-section .sidebar-section-more", { hasText: "All servers" }).click();
    await expect(page.locator(".game-server-list")).toContainText("No game servers added");
    await expect(page.locator(".game-server-add-form")).toBeVisible();
  });
});

test("a malformed welcome-screen answer is refused at the boundary; the events panel opens on the first click", async ({ page }) => {
  // handleLoadWelcomeScreen (actions/community/profile.ts) checks the
  // WelcomeScreen shape: [] stores no screen instead of throwing inside
  // WelcomeScreen.tsx's render, which cost the main pane its insert effect.
  await openCommunity(page, "[]");
  await page.locator(".sidebar-events-section .sidebar-section-more", { hasText: "All events" }).click();
  await expect(page.locator(".community-main .event-list")).toContainText("Community Events");
  await expect(page.locator(".welcome-screen")).toHaveCount(0);
});
