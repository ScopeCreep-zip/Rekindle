import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// "Publishing N changes" (GovernancePublicationStatus.tsx) is the Host's
// truth about our ops not yet landed (walk M86): a window opened after an
// op's `Pending` reads it from `get_community_details`
// (`CommunityView::op_publications`, ADR 0013: a new window gets no replay),
// and each later `opPublication` event moves it. The event's shape is the
// golden JSON `rekindle-types` serializes from the Rust type.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, { governance: { opPublication: Record<string, unknown> } }>;

const COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden sample's community
const OP_A = "a".repeat(64);
const OP_B = "b".repeat(64);

function handler(publications: string): string {
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
        roles: [{ id: "00000000000000000000000000000000", name: "@everyone", color: 0, permissions: "65536", position: 0,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${"e".repeat(64)}", myRoleIds: ["00000000000000000000000000000000"], myPermissions: "65536",
        myChannelPermissions: {}, opPublications: ${publications},
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

async function open(page: Page, publications: string): Promise<void> {
  await preloadMocks(page, "community", handler(publications));
  await openWindow(page, `/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
}

/** Deliver our op `op`'s publication `state` on the window's event channel. */
async function publication(page: Page, op: string, state: Record<string, unknown>): Promise<void> {
  const sample = GOLDEN["governance.opPublication"];
  const payload = { governance: { opPublication: { ...sample.governance.opPublication, op, state } } };
  await page.evaluate(
    ({ payload }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: 0,
        message: { cursor: null, channel: "community-event", payload },
      });
    },
    { payload },
  );
}

const pending = (page: Page) => page.locator(".governance-publication-pending");

test("a window opened after an op's Pending shows it, and clears it on Landed", async ({ page }) => {
  await open(page, `{ "${OP_A}": { kind: "pending" } }`);
  await expect(pending(page)).toHaveText("Publishing 1 change; it applies once the network accepts it");
  await publication(page, OP_A, { kind: "landed" });
  await expect(pending(page)).toHaveCount(0);
});

test("each live Pending clears on its Landed", async ({ page }) => {
  await open(page, "{}");
  await publication(page, OP_A, { kind: "pending" });
  await publication(page, OP_B, { kind: "pending" });
  await expect(pending(page)).toHaveText("Publishing 2 changes; they apply once the network accepts them");
  await publication(page, OP_A, { kind: "landed" });
  await expect(pending(page)).toHaveText("Publishing 1 change; it applies once the network accepts it");
  await publication(page, OP_B, { kind: "landed" });
  await expect(pending(page)).toHaveCount(0);
});
