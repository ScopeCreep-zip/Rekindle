import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// Community events for a community the window does not hold yet, and the
// backend's false-snapshot notice (plan E7.13 walk 13, E3.2 S7c.8). Payloads
// are the golden samples `rekindle-types` serializes from the Rust types.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const GOLDEN_COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden samples' community
const WINDOW_COMMUNITY = `VLD0:${"h".repeat(43)}`;
const CHANNEL = `ch_${"c".repeat(32)}`;
const ME = "e".repeat(64);

function detail(id: string, name: string): string {
  return `{
    id: "${id}", name: "${name}", description: null,
    channels: [{ id: "${CHANNEL}", name: "general", channelType: "text" }],
    roles: [{ id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0,
              hoist: false, mentionable: false }],
    myPseudonymKey: "${ME}", myRoleIds: [0], removed: null,
  }`;
}

// The Host lists the golden community only once `__joinedSecond` is set:
// until then the window hydrates without it, as one created or joined after
// the window opened.
const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Bea" };
    case "get_community_details":
      return window.__joinedSecond
        ? [${detail(WINDOW_COMMUNITY, "Lan Party")}, ${detail(GOLDEN_COMMUNITY, "Second Base")}]
        : [${detail(WINDOW_COMMUNITY, "Lan Party")}];
    case "get_welcome_screen":
      return { description: "", channels: [] };
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

let nextIndex = 0;

async function open(page: Page, community: string, joinedSecond = false): Promise<void> {
  nextIndex = 0;
  await preloadMocks(page, "community", HANDLER);
  if (joinedSecond) {
    await page.addInitScript(() => {
      (window as any).__joinedSecond = true;
    });
  }
  await page.goto(`/community?id=${community}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
}

async function deliver(page: Page, sample: string): Promise<void> {
  const payload = GOLDEN[sample];
  expect(payload, `golden sample ${sample}`).toBeDefined();
  const index = nextIndex++;
  await page.evaluate(
    ({ index, payload }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index,
        message: { cursor: null, channel: "community-event", payload },
      });
    },
    { index, payload },
  );
}

// Walk 13: `left` wrote under the missing row and threw; `memberDiscovered`
// was dropped. Either way the window never held the community it is in.
test("a member event for a community the window does not hold re-reads that community", async ({ page }) => {
  const failures: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error" && m.text().includes("community event slice")) failures.push(m.text());
  });
  await open(page, WINDOW_COMMUNITY);
  await page.evaluate(() => {
    (window as any).__joinedSecond = true;
  });
  await deliver(page, "membership.left");
  await expect
    .poll(() =>
      page.evaluate(
        (c) =>
          ((window as any).__ipcCalls as { cmd: string; args: Record<string, unknown> }[]).some(
            (x) => x.cmd === "get_community_members" && x.args?.communityId === c,
          ),
        GOLDEN_COMMUNITY,
      ),
    )
    .toBe(true);
  expect(failures).toEqual([]);
});

test("a false checkpoint snapshot shows a notice that stays", async ({ page }) => {
  await open(page, GOLDEN_COMMUNITY, true);
  await deliver(page, "governance.checkpointSnapshotMismatch");
  const notice = page.getByText(/did not match its full history/);
  await expect(notice).toHaveText(
    "The snapshot of Second Base your inviter's app sent did not match its full history. Second Base now uses the full history.",
  );
  // Later events do not clear it.
  await deliver(page, "membership.left");
  await page.waitForTimeout(500);
  await expect(notice).toBeVisible();
});
