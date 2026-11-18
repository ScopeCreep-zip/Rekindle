import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";

// Plan E7.13 walk row ST22: the quiet-hours controls are loaded by
// get_quiet_hours when the Notifications tab mounts. A user's edit made
// before that answer arrived was overwritten by it (the stored row came back
// disabled after the click enabled it).

const HANDLER = `
  const w = window;
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_quiet_hours":
      // The stored row (disabled), answered after the user's first click.
      return new Promise((r) => setTimeout(() => r({ enabled: false, startHour: 22, endHour: 8, timezone: "UTC" }), 800));
    case "set_quiet_hours":
      (w.__qhSets ??= []).push(args);
      return null;
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("the quiet-hours controls stay disabled until the stored hours load, then edit them", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER);
  await page.goto("/settings?tab=notifications");
  const box = page.locator("label.settings-option", { hasText: "Enable quiet hours" }).locator("input[type=checkbox]");
  const start = page.locator('input.form-input[type="number"]').first();
  await expect(box).toBeVisible();
  // Before get_quiet_hours answers (800 ms), no edit can be made that the
  // answer would then overwrite.
  await expect(box).toBeDisabled();
  await expect(box).toBeEnabled({ timeout: 5_000 });
  await expect(box).not.toBeChecked();
  await box.check();
  await expect(start).toBeEnabled();
  await expect.poll(() => page.evaluate(() => (window as any).__qhSets?.at(-1)?.enabled)).toBe(true);
});
