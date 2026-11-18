import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// The community window reads what we may do from the backend's permission
// flags (plan E3.2 S9): `myPermissions` and, for the open channel,
// `myChannelPermissions`; it computes no policy from roles. An
// `effectivePermissionsChanged` naming us re-reads them, so a remote
// permission change shows live. The event sample is the golden JSON
// `rekindle-types` serializes from the Rust type.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const COMMUNITY = `VLD0:${"g".repeat(43)}`; // the golden samples' community()
// The golden `governance.effectivePermissionsChanged` sample names `pk('p')`.
const ME = "p".repeat(64);
const SEND = "65536"; // VIEW_CHANNELS is bit 0, SEND_MESSAGES bit 16
const VIEW_ONLY = "1";
const DENIED = "You don't have permission to send messages";

const ADMINISTRATOR = (1n << 50n).toString();
const BOB = "b".repeat(64);
const CAROL = "c".repeat(64);

// A member row as the Host serves it, with its per-member verdicts.
const member = (key: string, name: string, may: object, outranks: boolean) => ({
  pseudonymKey: key, displayName: name, roleIds: ["00000000000000000000000000000000"], displayRole: "@everyone",
  status: "online", timeoutUntil: null, lastActive: 0, group: { kind: "online" },
  canModerateTarget: may, outranks,
});

// Bob's verdicts allow a ban only; Carol's nothing although we outrank her.
const MEMBERS = [
  member(BOB, "Bob", { kick: false, ban: true, timeout: false }, false),
  member(CAROL, "Carol", { kick: false, ban: false, timeout: false }, true),
];

// One event we created, starting tomorrow.
const EVENTS = `[{ id: "evt_${"1".repeat(32)}", title: "LAN night", description: "",
  creatorPseudonym: "${ME}", startTime: Math.floor(Date.now() / 1000) + 86400,
  createdAt: 0, status: "scheduled", rsvps: [] }]`;

// The roles grant SEND_MESSAGES (and our own role ADMINISTRATOR at the top
// rank), so only the backend's flags can deny anything.
function handler(channelPerms: string): string {
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
        roles: [{ id: "00000000000000000000000000000000", name: "@everyone", color: 0, permissions: "${SEND}", position: 0,
                  hoist: false, mentionable: false },
                { id: "01010101010101010101010101010101", name: "Admins", color: 0, permissions: "${ADMINISTRATOR}", position: 9,
                  hoist: false, mentionable: false }],
        myPseudonymKey: "${ME}", myRoleIds: ["00000000000000000000000000000000", "01010101010101010101010101010101"],
        myPermissions: window.__myPermissions ?? "${SEND}",
        myChannelPermissions: ${channelPerms}, removed: null,
      }];
    case "get_community_members":
      return ${JSON.stringify(MEMBERS)};
    case "get_events":
      return ${EVENTS};
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

async function open(page: Page, channelPerms: string): Promise<void> {
  await preloadMocks(page, "community", handler(channelPerms));
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await page.getByText("general", { exact: true }).first().click();
}

const composer = (page: Page) => page.getByPlaceholder("Type a message...");

test("a channel's flags deny posting the base allows", async ({ page }) => {
  await open(page, `{ ch1: "${VIEW_ONLY}" }`);
  await expect(page.getByText(DENIED)).toBeVisible();
  await expect(composer(page)).toHaveCount(0);
});

test("our effectivePermissionsChanged re-reads the flags live", async ({ page }) => {
  await open(page, "{}");
  await expect(composer(page)).toBeVisible();
  await page.evaluate(
    ({ payload, viewOnly }) => {
      const w = window as any;
      w.__myPermissions = viewOnly;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: 0,
        message: { cursor: null, channel: "community-event", payload },
      });
    },
    { payload: GOLDEN["governance.effectivePermissionsChanged"], viewOnly: VIEW_ONLY },
  );
  await expect(page.getByText(DENIED)).toBeVisible();
  await expect(composer(page)).toHaveCount(0);
});

test("a member's menu shows exactly the Host's verdicts for that member", async ({ page }) => {
  await open(page, "{}");
  const list = page.locator(".member-list");
  await list.getByText("Bob", { exact: true }).click({ button: "right" });
  const menu = page.locator(".context-menu").first();
  await expect(menu.getByText("Ban Member")).toBeVisible();
  await expect(menu.getByText("Kick Member")).toHaveCount(0);
  await expect(menu.getByText("Timeout")).toHaveCount(0);
  await expect(menu.getByText("Manage Roles")).toHaveCount(0);
  await page.keyboard.press("Escape");
  // We outrank Carol, but our served flags lack MANAGE_ROLES and her
  // verdicts are all false: only Block is offered.
  await list.getByText("Carol", { exact: true }).click({ button: "right" });
  const carol = page.getByRole("menu");
  await expect(carol.getByRole("menuitem", { name: / Block$/ })).toBeVisible();
  await expect(carol.getByText("Manage Roles")).toHaveCount(0);
  await expect(carol.getByText("Ban Member")).toHaveCount(0);
});

test("an event's actions follow the served MANAGE_EVENTS, not who created it", async ({ page }) => {
  await open(page, "{}");
  await page.getByText("All events").click();
  await expect(page.getByText("LAN night").last()).toBeVisible();
  await expect(page.getByText("Delete Event")).toHaveCount(0);
});
