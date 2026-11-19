import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { PEER, stressHandler, FULL } from "./fixtures/stress";
import { openWindow } from "./fixtures/paint";

// Plan E7.2 "Sharing totals with friends": Settings → Privacy's switch is a
// trust-changing intent the Host reviews (U1), off until its stored value
// loads; a friend's profile lists the totals they last shared.

const review = `{
  type: "intentRequired",
  data: {
    intentId: "01900000-0000-7000-8000-000000000072",
    kind: "shareGameTotals", class: "trustChanging",
    subject: { named: { name: "Ann" } },
    facts: [], escalations: [], reversibility: { reversibleByInverse: "shareGameTotals" },
    notifies: "nobody", proof: "commit", expiresAtMs: Date.now() + 60000, cancelSends: false,
  },
}`;

const SETTINGS = `
  window.__share ??= false;
  const prefs = () => ({
    notificationsEnabled: true, notificationSound: true, autoStart: false,
    gameDetectionEnabled: true, gameScanIntervalSecs: 30,
    readReceiptsEnabled: false, shareGameTotals: window.__share,
  });
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_preferences":
      return prefs();
    case "get_blocked_users":
      return [];
    case "set_share_game_totals":
      window.__asked = args.enabled;
      throw ${review};
    case "intent_commit":
      window.__share = window.__asked;
      return prefs();
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("sharing game time is off, and turning it on goes through the Host's review", async ({ page }) => {
  await preloadMocks(page, "settings", SETTINGS);
  await openWindow(page, "/settings?tab=privacy");
  const box = page.getByRole("checkbox", { name: "Share game time with friends" });
  await expect(box).toBeEnabled();
  await expect(box).not.toBeChecked();

  await box.click();
  await page.getByRole("button", { name: /^Share game totals: Ann/ }).click();
  await expect(box).toBeChecked();
  const calls = await page.evaluate(() => (window as any).__ipcCalls);
  expect(calls.find((c: { cmd: string }) => c.cmd === "set_share_game_totals").args).toEqual({ enabled: true });
});

test("a friend's profile lists the game time they shared, with its icon", async ({ page }) => {
  const totals = JSON.stringify([
    { gameId: 4181, gameName: "Counter-Strike 2", iconKey: "xfire/8507", totalSecs: 9000, lastPlayed: Date.now() - 120000 },
  ]);
  const handler = `if (cmd === "get_friend_game_totals") { window.__totalsFor = args.publicKey; return ${totals}; }\n${stressHandler(FULL)}`;
  await preloadMocks(page, `profile-${PEER.slice(0, 16)}`, handler);
  await openWindow(page, `/profile?key=${PEER}`);
  const rows = page.getByRole("list", { name: "Game time" }).getByRole("listitem");
  await expect(rows).toHaveCount(1);
  await expect(rows.nth(0)).toContainText("Counter-Strike 2");
  await expect(rows.nth(0)).toContainText("2h 30m");
  await expect(rows.nth(0).locator("img.game-icon")).toHaveAttribute("src", /8507/);
  expect(await page.evaluate(() => (window as any).__totalsFor)).toBe(PEER);
});

test("a friend who shares nothing shows no game time section", async ({ page }) => {
  const handler = `if (cmd === "get_friend_game_totals") return [];\n${stressHandler(FULL)}`;
  await preloadMocks(page, `profile-${PEER.slice(0, 16)}`, handler);
  await openWindow(page, `/profile?key=${PEER}`);
  await expect(page.locator(".profile-display-name")).toBeVisible();
  await expect(page.getByRole("list", { name: "Game time" })).toHaveCount(0);
});
