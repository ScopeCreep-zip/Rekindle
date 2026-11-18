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

// Plan E7.13 (the ST28 class): before get_quiet_hours answers, the controls
// show no value they have not read. They used to show 22, 7 and the OS zone.
const STORED_ON = `
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_quiet_hours":
      return new Promise((r) => setTimeout(() => r({ enabled: true, startHour: 23, endHour: 6, timezone: "UTC" }), 800));
    default:
      return null;
  }
`;

test("the quiet-hours controls show no value until the stored hours load", async ({ page }) => {
  await preloadMocks(page, "settings", STORED_ON);
  await page.goto("/settings?tab=notifications");
  const box = page.locator("label.settings-option", { hasText: "Enable quiet hours" }).locator("input[type=checkbox]");
  const hours = page.locator('input.form-input[type="number"]');
  const zone = page.locator("div.form-field", { hasText: "Timezone" }).locator("select.form-select");
  await expect(box).toBeVisible();
  // One reading before the answer: an expect() retries until it holds, so it
  // would pass on the loaded values.
  const shown = await page.evaluate(() => {
    const cb = [...document.querySelectorAll("label.settings-option")]
      .find((l) => l.textContent?.includes("Enable quiet hours"))
      ?.querySelector("input") as HTMLInputElement;
    const nums = [...document.querySelectorAll('input.form-input[type="number"]')] as HTMLInputElement[];
    const sels = [...document.querySelectorAll("select.form-select")] as HTMLSelectElement[];
    return {
      box: { checked: cb.checked, disabled: cb.disabled },
      hours: nums.map((n) => ({ value: n.value, disabled: n.disabled })),
      zone: sels[sels.length - 1]?.value,
    };
  });
  expect(shown).toEqual({
    box: { checked: false, disabled: true },
    hours: [
      { value: "", disabled: true },
      { value: "", disabled: true },
    ],
    zone: "",
  });
  await expect(box).toBeChecked({ timeout: 5_000 });
  await expect(hours.nth(0)).toHaveValue("23");
  await expect(hours.nth(1)).toHaveValue("6");
  await expect(hours.nth(0)).toBeEnabled();
  await expect(zone).toHaveValue("UTC");
});
