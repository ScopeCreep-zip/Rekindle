import { test, expect, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Choosing a status closes the picker, by pointer and by keyboard (walk
// B63). Kobalte's radio items keep their menu open unless `closeOnSelect`
// is set, which left the dropdown open over the buddy list and swallowed
// the next press (B9's Calls menu).

const ALICE = "a".repeat(64);

const HANDLER = `
  switch (cmd) {
    case "get_identity":
      return { publicKey: "${ALICE}", displayName: "Alice", status: "online" };
    case "set_status":
      return { status: args.status };
    case "get_friends":
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

async function open(page: Page): Promise<void> {
  await preloadMocks(page, "buddy-list", HANDLER);
  await page.goto("/buddy-list");
  await expect(page.locator(".status-picker")).toBeVisible({ timeout: 10_000 });
}

function statusCalls(page: Page): Promise<unknown[]> {
  return page.evaluate(() =>
    ((window as any).__ipcCalls ?? [])
      .filter((c: { cmd: string }) => c.cmd === "set_status")
      .map((c: { args: unknown }) => c.args),
  );
}

test("a pointer choice sets the status and closes the picker", async ({ page }) => {
  await open(page);
  await page.locator(".status-picker").click();
  const dropdown = page.locator(".status-picker-dropdown");
  await expect(dropdown).toBeVisible();
  await dropdown.locator(".context-menu-item", { hasText: "Away" }).click();
  await expect(dropdown).toHaveCount(0);
  await expect.poll(() => statusCalls(page)).toEqual([{ status: "away" }]);
  await expect(page.locator(".status-picker .buddy-name")).toHaveText("Away");
});

test("a keyboard choice sets the status and closes the picker", async ({ page }) => {
  await open(page);
  await page.locator(".status-picker").focus();
  await page.keyboard.press("Enter");
  const dropdown = page.locator(".status-picker-dropdown");
  // Kobalte moves focus into the menu after it opens: a key pressed before
  // that lands on the trigger, so wait for the first item to hold focus.
  const items = dropdown.locator(".context-menu-item");
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dropdown).toHaveCount(0);
  await expect.poll(() => statusCalls(page)).toHaveLength(1);
});
