import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.2 "Game-time tracking, persisted": Settings → Profile shows the
// Host's totals per game (`get_game_totals` → `GameTotalsGet`), with the
// last played and the icon the backend resolved; a key with no file shows
// the generic glyph.

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_game_totals":
      return [
        { gameId: 4181, gameName: "Counter-Strike 2", iconKey: "xfire/8507", totalSecs: 9000, lastPlayed: Date.now() - 120000 },
        { gameId: 4900, gameName: "Valorant", iconKey: "slug/valorant", totalSecs: 1800, lastPlayed: Date.now() - 3 * 86400000 },
      ];
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("Settings → Profile lists each game's total, last played and icon", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER);
  await openWindow(page, "/settings?tab=profile");
  const rows = page.getByRole("list", { name: "Game time" }).getByRole("listitem");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("Counter-Strike 2");
  await expect(rows.nth(0)).toContainText("2h 30m");
  await expect(rows.nth(0)).toContainText("2m ago");
  await expect(rows.nth(0).locator("img.game-icon")).toHaveAttribute("src", /8507/);
  await expect(rows.nth(1)).toContainText("Valorant");
  await expect(rows.nth(1)).toContainText("30m");
  await expect(rows.nth(1).locator("img.game-icon")).toHaveCount(0);
  await expect(rows.nth(1).locator(".game-icon-generic")).toBeVisible();
});

test("Settings → Profile says so when no game was played", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER.replace(/case "get_game_totals":[\s\S]*?\];/, 'case "get_game_totals": return [];'));
  await openWindow(page, "/settings?tab=profile");
  await expect(page.getByText("No games played yet")).toBeVisible();
});
