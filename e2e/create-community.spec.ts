import { test, expect, type Page } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Walk 15, B60: a create committed before the node attached was refused
// (409 "cannot perform write operation in state 'detached'"); the form
// closed as if it had worked and the toast dropped the Host's reason.

const CONNECTED = `{ attachmentState: "attached_good", isAttached: true, publicInternetReady: true, hasRoute: true, mediaRoute: "available", profileDhtKey: null, friendListDhtKey: null }`;
const ATTACHING = `{ attachmentState: "attaching", isAttached: false, publicInternetReady: false, hasRoute: false, mediaRoute: "idle", profileDhtKey: null, friendListDhtKey: null }`;

const handler = (network: string) => `
  switch (cmd) {
    case "get_network_status":
      return ${network};
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
  await preloadMocks(page, "buddy-list", handler(CONNECTED));
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

async function sendNetwork(page: Page, index: number, status: Record<string, unknown>): Promise<void> {
  await page.evaluate(({ index, status }) => {
    const w = window as any;
    w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
      index,
      message: { cursor: null, channel: "network-status", payload: { network: { attachmentChanged: status } } },
    });
  }, { index, status });
}

// Decided after walk 15: a community create is a durable DHT write, so
// Create waits for the indicator's "Connected" (like Element, which refuses
// room creation offline), and says why while it waits.
test("Create waits for the network and says why", async ({ page }) => {
  await preloadMocks(page, "buddy-list", handler(ATTACHING));
  await page.goto("/buddy-list");
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.locator(".network-indicator span")).toHaveText("Connecting...");
  await page.locator('.buddy-tab[title^="Communities"]').click();
  const create = page.locator('[aria-label="Create Community"]');
  await expect(create).toBeDisabled();
  await expect(create).toHaveAttribute("title", "Create Community (waiting for the network: Connecting...)");

  await sendNetwork(page, 0, {
    attachmentState: "attached_good", isAttached: true, publicInternetReady: true,
    hasRoute: true, mediaRoute: "available",
  });
  await expect(page.locator(".network-indicator span")).toHaveText("Connected");
  await expect(create).toBeEnabled();
  await create.click();
  await page.locator('input[placeholder="Community name..."]').fill("lan party");
  const submit = page.locator('.modal-container button[type="submit"]');
  await expect(submit).toBeEnabled();

  // The network drops while the form is open: the submit waits again.
  await sendNetwork(page, 1, {
    attachmentState: "attaching", isAttached: false, publicInternetReady: false,
    hasRoute: false, mediaRoute: "idle",
  });
  await expect(submit).toBeDisabled();
  await expect(page.locator(".modal-container .form-hint")).toHaveText(
    "Waiting for the network (Connecting...): a community can be created once connected.",
  );
});
