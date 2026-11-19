import { test, expect, type Locator, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { COMMUNITY, FULL, stressHandler } from "./fixtures/stress";
import { openWindow } from "./fixtures/paint";

// Plan E7.3, wiring guide §13c and §13f: every modal dialog keeps Tab and
// Shift+Tab inside itself, Esc closes it, and focus goes back to the
// control that opened it.

/** The stress community, with server ids on its channel messages so a
 *  message offers Forward. */
const COMMUNITY_HANDLER = `
  const value = (function (cmd, args) { ${stressHandler(FULL)} })(cmd, args);
  if (cmd === "get_channel_messages") {
    return value.map((m) => ({ ...m, serverMessageId: "msg_" + String(m.id).padStart(32, "0") }));
  }
  return value;
`;

/** Tab and Shift+Tab many times round the dialog: focus never leaves it. */
async function expectFocusTrapped(page: Page, dialog: Locator): Promise<void> {
  // The dialog moves focus in once it has mounted.
  await expect.poll(() => dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  for (const key of ["Tab", "Shift+Tab"]) {
    for (let i = 0; i < 14; i++) {
      await page.keyboard.press(key);
      const inside = await dialog.evaluate((el) => el.contains(document.activeElement));
      expect(inside, `${key} #${i + 1} stays in the dialog`).toBe(true);
    }
  }
}

async function openCommunity(page: Page): Promise<void> {
  await preloadMocks(page, "community", COMMUNITY_HANDLER);
  await openWindow(page, `/community?id=${COMMUNITY}`);
  await page.getByText("channel-0-", { exact: false }).first().click();
  await expect(page.getByText("message 1 with some ordinary words")).toBeVisible();
}

test("the search panel traps focus, Esc closes it, focus returns to Search", async ({ page }) => {
  await openCommunity(page);
  const opener = page.getByRole("button", { name: "Search community" });
  await opener.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Search messages" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByPlaceholder("Search…")).toBeFocused();
  await expectFocusTrapped(page, dialog);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("the forward dialog traps focus, Esc closes it, focus returns to Forward", async ({ page }) => {
  await openCommunity(page);
  const message = page.locator(".chat-message", { hasText: "message 1 with some ordinary words" });
  await message.hover();
  const opener = message.getByRole("button", { name: "Forward message" });
  await opener.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Forward message" });
  await expect(dialog).toBeVisible();
  await expectFocusTrapped(page, dialog);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("a shared Modal (Add Friend) traps focus, Esc closes it, focus returns", async ({ page }) => {
  await preloadMocks(page, "buddy-list", stressHandler(FULL));
  await openWindow(page, "/buddy-list");
  const opener = page.getByRole("button", { name: "Add Friend" });
  await opener.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expectFocusTrapped(page, dialog);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});
