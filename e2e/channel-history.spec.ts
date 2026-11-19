import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { COMMUNITY, FULL, stressHandler } from "./fixtures/stress";
import { openWindow } from "./fixtures/paint";

// Channel history is a Host call (`get_channel_messages` → ChannelMessages,
// `Done<Vec<Message>>` since E3.1): the mocks answer it wrapped as the shell
// does, and the community window must unwrap it and draw the messages.

test("channel history answered as the Host's Done renders its messages", async ({ page }) => {
  await preloadMocks(page, "community", stressHandler(FULL));
  await openWindow(page, `/community?id=${COMMUNITY}`);
  await page.getByText("channel-0-", { exact: false }).first().click();
  await expect(page.getByText("message 1 with some ordinary words")).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__ipcCalls.map((c: { cmd: string }) => c.cmd));
  expect(calls).toContain("get_channel_messages");
});
