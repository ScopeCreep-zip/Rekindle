import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { COMMUNITY, FULL, stressHandler } from "./fixtures/stress";

// A post sent before the community key arrived is queued by the Host, not
// refused (plan E3.2 S7l, F9-1): the send answers `awaiting_key` and the
// bubble says so until the key comes.

test("a post sent before the community key shows it is waiting for the key", async ({ page }) => {
  const handler =
    `if (cmd === "send_channel_message") return { messageId: "msg_waiting", status: "awaiting_key" };\n` +
    stressHandler(FULL);
  await preloadMocks(page, "community", handler);
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.getByText("channel-0-", { exact: false }).first().click();
  const input = page.getByPlaceholder("Type a message...");
  await input.fill("posted before the key");
  await input.press("Enter");
  await expect(page.getByText("posted before the key")).toBeVisible();
  const note = page.locator(".message-status-note").filter({ hasText: "Waiting for the community key" });
  await expect(note).toBeVisible();
  // Not an error: no failure toast for a post the Host holds.
  await expect(page.getByText("Failed to send message")).toHaveCount(0);
});
