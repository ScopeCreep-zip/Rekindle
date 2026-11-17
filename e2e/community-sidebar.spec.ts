import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Plan E7.13 walk finding W6: the sidebar's links are the only way to the
// events panel and the game-server list (with its Add form). They were drawn
// only once an upcoming event existed, or more than three servers, so a new
// community could never create its first event or add its first server.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const ME = "e".repeat(64);
// MANAGE_COMMUNITY (1 << 3) | SEND_MESSAGES (1 << 16), as decimal strings.
const PERMS = String((1n << 3n) | (1n << 16n));

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Bea" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [{ id: "ch1", name: "general", channelType: "text" }],
        roles: [{ id: 0, name: "@everyone", color: 0, permissions: "${PERMS}", position: 0,
          hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: [0], removed: null,
      }];
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

test.beforeEach(async ({ page }) => {
  await preloadMocks(page, "community", HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  // The window selects the community (closing both panels) once hydrated;
  // the channel header is drawn only after that selection.
  await expect(page.locator(".community-channel-header-name")).toHaveText("general");
});

test("with no events, the Upcoming section still offers All events", async ({ page }) => {
  // Opening the panel from here is a separate open finding (walk W6b): the
  // first switch from a channel mounts EventsPanel without inserting it.
  const link = page.locator(".sidebar-events-section .sidebar-section-more", { hasText: "All events" });
  await expect(link).toBeVisible();
});

test("with no game servers, All servers opens the list and its Add form", async ({ page }) => {
  await expect(page.locator(".sidebar-servers-section")).toContainText("Servers (0)");
  await page.locator(".sidebar-servers-section .sidebar-section-more", { hasText: "All servers" }).click();
  await expect(page.locator(".game-server-list")).toContainText("No game servers added");
  await expect(page.locator(".game-server-add-form")).toBeVisible();
});
