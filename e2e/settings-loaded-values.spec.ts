import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";

// Plan E7.13 walk rows ST21 and ST28: a Settings box shows only a value it
// has read. "Generate previews" started ticked before its stored value (off)
// loaded, and "Read receipts" started unticked and enabled, so the walk read
// the initial `false` back after reopening Settings (ST28). Each stays
// unticked and disabled until its stored value answers (800 ms here).

const PREFS = `{
  notificationsEnabled: true,
  notificationSound: true,
  startMinimized: false,
  autoStart: false,
  gameDetectionEnabled: true,
  gameScanIntervalSecs: 30,
  readReceiptsEnabled: true,
}`;

const HANDLER = `
  const later = (v) => new Promise((r) => setTimeout(() => r(v), 800));
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_link_previews_enabled":
      return later(false);
    case "get_preferences":
      return later(${PREFS});
    case "get_blocked_users":
      return [];
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

function before(box: import("@playwright/test").Locator) {
  return box.evaluate((el: HTMLInputElement) => ({ disabled: el.disabled, checked: el.checked }));
}

function option(page: import("@playwright/test").Page, text: string) {
  return page.locator("label.settings-option", { hasText: text }).locator("input[type=checkbox]");
}

test("Generate previews is unticked and disabled until its stored value (off) loads", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER);
  await page.goto("/settings?tab=notifications");
  const box = option(page, "Generate previews");
  await expect(box).toBeVisible();
  // One reading before the answer: an expect() retries until it holds, so it
  // would pass on the loaded value.
  expect(await before(box)).toEqual({ disabled: true, checked: false });
  await expect(box).toBeEnabled({ timeout: 5_000 });
  await expect(box).not.toBeChecked();
});

test("Read receipts is unticked and disabled until its stored value (on) loads", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER);
  await page.goto("/settings?tab=privacy");
  const box = option(page, "Read receipts");
  await expect(box).toBeVisible();
  // One reading before the answer: an expect() retries until it holds, so it
  // would pass on the loaded value.
  expect(await before(box)).toEqual({ disabled: true, checked: false });
  await expect(box).toBeEnabled({ timeout: 5_000 });
  await expect(box).toBeChecked();
});
