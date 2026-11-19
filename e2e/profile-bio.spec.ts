import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { PEER, stressHandler, FULL } from "./fixtures/stress";
import { openWindow } from "./fixtures/paint";

// Plan E7.2 "Profile bio": Settings → Profile edits our bio (190
// characters, counted as the Host counts them), saved through the Host;
// a friend's profile shows the bio they sent as plain text.

const SETTINGS = `
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_bio":
      return "Quake 3 since '99";
    case "get_game_totals":
      return [];
    case "set_bio":
      window.__bio = args.bio;
      return args.bio.trim() === "" ? null : args.bio;
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("our bio loads, is capped at 190 characters, and saves through the Host", async ({ page }) => {
  await preloadMocks(page, "settings", SETTINGS);
  await openWindow(page, "/settings?tab=profile");
  const box = page.getByRole("textbox", { name: "Bio" });
  await expect(box).toHaveValue("Quake 3 since '99");
  const save = page.locator("#bio-count + .form-btn-primary");
  await expect(page.locator("#bio-count")).toHaveText("17 of 190 characters");

  // 190 emoji are 380 UTF-16 units but 190 characters: allowed.
  await box.fill("🔥".repeat(190));
  await expect(page.locator("#bio-count")).toHaveText("190 of 190 characters");
  await expect(save).toBeEnabled();
  await box.fill("x".repeat(191));
  await expect(save).toBeDisabled();

  await box.fill("LAN parties forever");
  await save.click();
  await expect.poll(() => page.evaluate(() => (window as any).__bio)).toBe("LAN parties forever");
});

test("a friend's bio shows as plain text on their profile", async ({ page }) => {
  const bio = JSON.stringify("<b>not bold</b>\nsecond line");
  const handler = `if (cmd === "get_friend_bio") { window.__bioFor = args.publicKey; return ${bio}; }\n${stressHandler(FULL)}`;
  await preloadMocks(page, `profile-${PEER.slice(0, 16)}`, handler);
  await openWindow(page, `/profile?key=${PEER}`);
  const shown = page.locator(".profile-bio");
  await expect(shown).toHaveText("<b>not bold</b>\nsecond line");
  await expect(shown.locator("b")).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__bioFor)).toBe(PEER);
});

test("a friend with no bio shows no bio section", async ({ page }) => {
  const handler = `if (cmd === "get_friend_bio") return null;\n${stressHandler(FULL)}`;
  await preloadMocks(page, `profile-${PEER.slice(0, 16)}`, handler);
  await openWindow(page, `/profile?key=${PEER}`);
  await expect(page.locator(".profile-display-name")).toBeVisible();
  await expect(page.locator(".profile-bio")).toHaveCount(0);
});
