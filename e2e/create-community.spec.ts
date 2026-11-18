import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Walk 15, B60: a create committed before the node attached was refused
// (409 "cannot perform write operation in state 'detached'"); the form
// closed as if it had worked and the toast dropped the Host's reason.

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Alice" };
    case "create_community":
      throw {
        type: "intentRequired",
        data: {
          intentId: "01900000-0000-7000-8000-000000000060",
          kind: "createCommunity", class: "trustChanging",
          subject: { named: { name: args.name } },
          facts: [], escalations: [], reversibility: "never", notifies: "nobody",
          proof: "commit", expiresAtMs: Date.now() + 60000, cancelSends: false,
        },
      };
    case "intent_commit":
      throw {
        type: "failed",
        data: {
          code: 409,
          message: "cannot perform write operation in state 'detached'",
          remediation: "wait for the daemon to reach operational state",
        },
      };
    case "get_friends":
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

test("a refused create keeps the form open with the Host's reason", async ({ page }) => {
  await preloadMocks(page, "buddy-list", HANDLER);
  await page.goto("/buddy-list");
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.locator('.buddy-tab[title^="Communities"]').click();
  await page.locator('[aria-label="Create Community"]').click();
  const name = page.locator('input[placeholder="Community name..."]');
  await name.fill("lan party");
  await page.locator('.modal-container button[type="submit"]').click();
  await page.locator(".intent-dialog .sec-btn-commit").click();

  await expect(page.locator(".intent-dialog")).toBeHidden();
  await expect(name).toBeVisible();
  await expect(page.getByRole("alert")).toHaveText(
    "cannot perform write operation in state 'detached' — wait for the daemon to reach operational state",
  );
});
