import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { firstPaint } from "./fixtures/paint";

// Plan E7.1 "Quick wipe": Settings → Privacy arms a duress passphrase and
// an optional panic shortcut through the Host's Protected review (U1),
// which lists exactly what a trigger destroys and asks for the identity's
// passphrase; disarming is the same review.

const DESTROYED =
  "the quick wipe. A trigger (the duress passphrase at any passphrase prompt, or the panic " +
  "shortcut) deletes every identity on this device with its messages, keys and files, the " +
  "settings, logs and skins, Rekindle's network storage and the window data, and closes " +
  "Rekindle. It cannot be undone.";

const review = (kind: string) => `{
  type: "intentRequired",
  data: {
    intentId: "01900000-0000-7000-8000-000000000071",
    kind: "${kind}", class: "protected",
    subject: { named: { name: ${JSON.stringify(DESTROYED)} } },
    facts: [], escalations: [], reversibility: "never",
    notifies: "nobody", proof: "passphrase", expiresAtMs: Date.now() + 60000, cancelSends: false,
  },
}`;

const SETTINGS = `
  (window.__calls = window.__calls || []).push(cmd);
  switch (cmd) {
    case "subscribe_events":
      return null;
    case "get_preferences":
      return { notificationsEnabled: true, notificationSound: true, autoStart: false,
        gameDetectionEnabled: true, gameScanIntervalSecs: 30, readReceiptsEnabled: false,
        shareGameTotals: false, shareCurrentGame: false };
    case "get_blocked_users":
    case "get_hidden_games":
    case "get_game_totals":
      return [];
    case "quick_wipe_arm":
      window.__armed = args;
      throw ${review("armQuickWipe")};
    case "quick_wipe_disarm":
      throw ${review("disarmQuickWipe")};
    case "intent_commit":
      window.__proof = args;
      return { armed: !!window.__armed && !window.__disarming,
        shortcut: window.__armed ? window.__armed.shortcut : null };
    default:
      return null;
  }
`;

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("arming the quick wipe reviews what it destroys and asks for the passphrase", async ({ page }) => {
  await preloadMocks(page, "settings", SETTINGS);
  await page.goto("/settings?tab=privacy");
  await firstPaint(page);
  const arm = page.getByRole("button", { name: "Arm Quick Wipe", exact: true });
  await expect(arm).toBeDisabled();
  await page.getByLabel("Duress passphrase", { exact: true }).fill("let it burn");
  await page.getByLabel("Confirm duress passphrase").fill("let it bur");
  await expect(page.getByText("The passphrases do not match.")).toBeVisible();
  await expect(arm).toBeDisabled();
  await page.getByLabel("Confirm duress passphrase").fill("let it burn");
  await page.getByLabel("Panic shortcut").fill("CommandOrControl+Shift+F12");
  await arm.click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("deletes every identity on this device");
  await expect(dialog).toContainText("It cannot be undone.");
  const commit = dialog.locator("[data-role=destructive], .sec-btn-commit").first();
  await expect(commit).toBeDisabled();
  await dialog.locator("input[type=password]").fill("my real passphrase");
  await commit.click();
  await expect(page.locator(".settings-content").getByRole("status")).toContainText("Quick wipe armed (shortcut CommandOrControl+Shift+F12).");
  expect(await page.evaluate(() => (window as any).__armed)).toEqual({
    duressPassphrase: "let it burn",
    shortcut: "CommandOrControl+Shift+F12",
  });
  await expect(page.getByLabel("Duress passphrase", { exact: true })).toHaveValue("");
});

test("disarming goes through the same review", async ({ page }) => {
  await preloadMocks(page, "settings", SETTINGS);
  await page.goto("/settings?tab=privacy");
  await firstPaint(page);
  await page.evaluate(() => { (window as any).__disarming = true; });
  await page.getByRole("button", { name: "Disarm Quick Wipe", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("input[type=password]").fill("my real passphrase");
  await dialog.locator("[data-role=destructive], .sec-btn-commit").first().click();
  await expect(page.locator(".settings-content").getByRole("status")).toContainText("Quick wipe disarmed.");
});

test("the section never shows whether the quick wipe is armed", async ({ page }) => {
  await preloadMocks(page, "settings", SETTINGS);
  await page.goto("/settings?tab=privacy");
  await firstPaint(page);
  await expect(page.getByRole("button", { name: "Arm Quick Wipe", exact: true })).toBeVisible();
  // Nothing asks the Host for the state: the vault header does not reveal it.
  const calls: string[] = await page.evaluate(() => (window as any).__calls ?? []);
  expect(calls.filter((c) => c.startsWith("quick_wipe"))).toEqual([]);
  // The fields start empty and no status line reports armed or disarmed.
  await expect(page.getByLabel("Duress passphrase", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Panic shortcut")).toHaveValue("");
  await expect(page.locator(".settings-content").getByRole("status")).toHaveCount(0);
  await expect(page.locator(".settings-content")).not.toContainText(/\b(dis)?armed\b/i);
});
