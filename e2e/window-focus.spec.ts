import { test, expect } from "@playwright/test";
import { preloadMocks, clearMocks, LOGIN_SUCCESS_HANDLER } from "./fixtures/mocks";

// Plan E7.7.2: every window marks `:root` with `data-window-inactive` while
// it lacks focus, so the generated region rules show a skin's
// window-inactive images (`src/styles/generated/regions.css`).

test.describe("Window focus", () => {
  test.afterEach(async ({ page }) => {
    await clearMocks(page);
  });

  test("the root follows the window's focus", async ({ page }) => {
    await preloadMocks(page, "login", LOGIN_SUCCESS_HANDLER);
    await page.goto("/login");
    await page.waitForSelector(".login-title", { timeout: 10_000 });

    const root = page.locator("html");
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(root).toHaveAttribute("data-window-inactive", "");
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(root).not.toHaveAttribute("data-window-inactive");
  });
});
