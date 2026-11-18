import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks } from "./fixtures/mocks";

// The community member list renders the groups the Host decided
// (`CommunityMemberView.group`; the rules are tested in rekindle-types
// `dto::member_groups`): a header wherever the group changes, in the Host's
// order, with the group's count. The list does no grouping of its own, so a
// row the Host put under "Online" stays there whatever status its dot draws.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const ME = "e".repeat(64);

type Group = { kind: "role"; id: string; name: string } | { kind: "online" } | { kind: "offline" };
const ADMINS: Group = { kind: "role", id: "01010101010101010101010101010101", name: "Admins" };
const ONLINE: Group = { kind: "online" };
const OFFLINE: Group = { kind: "offline" };

const member = (n: number, name: string, status: string, admin: boolean, group: Group) => ({
  pseudonymKey: n === 0 ? ME : n.toString(16).padStart(64, "d"),
  displayName: name,
  roleIds: admin ? ["00000000000000000000000000000000", "01010101010101010101010101010101"] : ["00000000000000000000000000000000"],
  displayRole: admin ? "Admins" : "@everyone",
  status,
  timeoutUntil: null,
  lastActive: 0,
  group,
});

// The Host's order: hoisted role group, then Online, then Offline.
// "HostSaysOnline" is offline by status but the Host put it under Online.
const MEMBERS = [
  member(0, "AdminOnline", "online", true, ADMINS),
  member(1, "AdminBusy", "busy", true, ADMINS),
  member(3, "UserOnline", "online", false, ONLINE),
  member(4, "UserAway", "away", false, ONLINE),
  member(5, "HostSaysOnline", "offline", false, ONLINE),
  member(2, "AdminOffline", "offline", true, OFFLINE),
  member(7, "UserOffline", "offline", false, OFFLINE),
];

const HANDLER = `
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
        roles: [
          { id: "00000000000000000000000000000000", name: "@everyone", color: 0, permissions: "65536", position: 0,
            hoist: false, mentionable: false },
          { id: "01010101010101010101010101010101", name: "Admins", color: 0, permissions: "65536", position: 1,
            hoist: true, mentionable: false },
        ],
        myPseudonymKey: "${ME}", myRoleIds: ["00000000000000000000000000000000", "01010101010101010101010101010101"], myPermissions: "65536", myChannelPermissions: {}, removed: null,
      }];
    case "get_community_members":
      return ${JSON.stringify(MEMBERS)};
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

test("member list renders the Host's groups in the Host's order", async ({ page }) => {
  await preloadMocks(page, "community", HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  const list = page.locator(".member-list");
  await expect(list.locator(".member-list-header")).toHaveText("Members — 7");
  await expect(list.locator(".member-group-header")).toHaveText([
    "Admins — 2",
    "Online — 3",
    "Offline — 2",
  ]);
  await expect(list.locator(".member-name")).toHaveText([
    "AdminOnline",
    "AdminBusy",
    "UserOnline",
    "UserAway",
    "HostSaysOnline",
    "AdminOffline",
    "UserOffline",
  ]);
});

// A member's presence or card can reach the window before its list holds the
// member (plan E7.13, B61): the window re-reads the list once for that
// member, as a `Joined` does, instead of dropping the event. The payloads are
// the golden samples `rekindle-types` serializes from the Rust types.
const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, Record<string, Record<string, Record<string, unknown>>>>;

const PEER = "a".repeat(64); // the golden presence sample's pseudonym
const PEER_ROW = { ...member(9, "Peer", "online", false, ONLINE), pseudonymKey: PEER };

// The Host lists only us until `window.__hostListsPeer` is set; every read
// is counted in `window.__memberReads`.
const LATE_HANDLER = HANDLER.replace(
  `return ${JSON.stringify(MEMBERS)};`,
  `window.__memberReads = (window.__memberReads ?? 0) + 1;
      return window.__hostListsPeer
        ? ${JSON.stringify([MEMBERS[0], PEER_ROW])}
        : ${JSON.stringify([MEMBERS[0]])};`,
);

async function openLate(page: Page): Promise<void> {
  await preloadMocks(page, "community", LATE_HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  await expect(page.locator(".member-list .member-name")).toHaveText(["AdminOnline"]);
}

async function send(page: Page, channel: string, payload: unknown): Promise<void> {
  await page.evaluate(
    ({ channel, payload }) => {
      const w = window as any;
      w.__hostListsPeer = true;
      w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, {
        index: 0,
        message: { cursor: null, channel, payload },
      });
    },
    { channel, payload },
  );
}

const reads = (page: Page) => page.evaluate(() => (window as any).__memberReads as number);

test("presence for a member the list lacks re-reads the list once", async ({ page }) => {
  await openLate(page);
  const before = await reads(page);
  await send(page, "presence-event", GOLDEN["presence.communityMemberChanged"]);
  await send(page, "presence-event", GOLDEN["presence.communityMemberChanged"]);
  await expect(page.locator(".member-list .member-name")).toHaveText(["AdminOnline", "Peer"]);
  expect(await reads(page)).toBe(before + 1);
});

test("a card for a member the list lacks re-reads the list", async ({ page }) => {
  await openLate(page);
  const card = GOLDEN["membership.memberCardChanged"];
  const payload = {
    membership: { memberCardChanged: { ...card.membership.memberCardChanged, pseudonym: PEER } },
  };
  await send(page, "community-event", payload);
  await expect(page.locator(".member-list .member-name")).toHaveText(["AdminOnline", "Peer"]);
});

// The owner's menu offers every moderation item the Host allows (plan
// E7.13, walk 19 c: the window once ranked the owner itself, casting the
// Owner role's `u32::MAX` position to -1, and hid the items). The window
// ranks no one now (plan E3.2 S9): our served flags hold ADMINISTRATOR and
// AdminBusy's row carries the Host's verdicts, which the validator computes
// (`rekindle-governance` `validate::target`, tested there and in the Host's
// `views_tests`).
const OWNER_HANDLER = HANDLER.replace(
  'myPermissions: "65536"',
  'myPermissions: "1125899906842624"',
).replace(
  '"displayName":"AdminBusy","roleIds":["00000000000000000000000000000000","01010101010101010101010101010101"],"displayRole":"Admins","status":"busy","timeoutUntil":null,"lastActive":0,"group":{"kind":"role","id":"01010101010101010101010101010101","name":"Admins"}',
  '"displayName":"AdminBusy","roleIds":["00000000000000000000000000000000","01010101010101010101010101010101"],"displayRole":"Admins","status":"busy","timeoutUntil":null,"lastActive":0,"group":{"kind":"role","id":"01010101010101010101010101010101","name":"Admins"},"canModerateTarget":{"kick":true,"ban":true,"timeout":true},"outranks":true',
);

test("the owner's menu on a member with a role offers every moderation item", async ({ page }) => {
  expect(OWNER_HANDLER).toContain('"outranks":true');
  expect(OWNER_HANDLER).toContain('myPermissions: "1125899906842624"');
  await preloadMocks(page, "community", OWNER_HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  await page.locator(".member-list .member-name", { hasText: "AdminBusy" }).click({ button: "right" });
  const menu = page.getByRole("menu");
  for (const item of ["Manage Roles", "Timeout", "Kick Member", "Ban Member"]) {
    // Each item's name is its icon glyph, a space, then the label.
    await expect(menu.getByRole("menuitem", { name: new RegExp(` ${item}$`) })).toBeVisible();
  }
});
