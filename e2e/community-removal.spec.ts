import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// Our own removal renders in the community window from the backend's
// fields (plan E3.2 S9, A/B R5-2): the stored `removed` state at load, so it
// survives a restart, and the live `banned`/`kicked` event, whose sample is
// the golden JSON `rekindle-types` serializes from the Rust type.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden samples' community()
// The golden `membership.*` samples name `pk('e')` as the target.
const ME = "e".repeat(64);

function handler(removed: string): string {
  return `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Bea" };
    case "get_community_details":
      return [{
        id: "${COMMUNITY}", name: "Lan Party", description: null,
        channels: [{ id: "ch1", name: "general", channelType: "text" }],
        // @everyone may post (SEND_MESSAGES = 1 << 16), so only the removal
        // disables the composer.
        roles: [{ id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: [0], myPermissions: "65536", myChannelPermissions: {}, removed: ${removed},
      }];
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;
}

async function open(page: Page, removed: string): Promise<void> {
  await preloadMocks(page, "community", handler(removed));
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
}

const banner = (page: Page) => page.locator(".removal-banner");

test("a stored removal renders the banner and disables posting", async ({ page }) => {
  await open(page, `{ banned: false, by: "${"f".repeat(64)}", byName: "Ann", reason: "spam" }`);
  await expect(banner(page)).toHaveText("You were removed from Lan Party by Ann: spam");
  await expect(page.getByPlaceholder("Type a message...")).toHaveCount(0);
});

test("a removal without a name or reason leaves those parts out", async ({ page }) => {
  await open(page, `{ banned: false, by: null, byName: null, reason: null }`);
  await expect(banner(page)).toHaveText("You were removed from Lan Party");
});

test("the live banned event renders the banner with its reason", async ({ page }) => {
  await open(page, "null");
  await expect(banner(page)).toHaveCount(0);
  await expect(page.getByPlaceholder("Type a message...")).toBeVisible();
  await page.evaluate(
    ({ payload }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: 0,
        message: { cursor: null, channel: "community-event", payload },
      });
    },
    { payload: GOLDEN["membership.banned"] },
  );
  await expect(banner(page)).toHaveText("You were banned from Lan Party by Mod: spam");
  await expect(page.getByPlaceholder("Type a message...")).toHaveCount(0);
});
