import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// The community member list groups by presence the way Discord's does:
// hoisted role groups hold only members who are not offline, the other
// present members (online, away, busy, in game) fall under "Online", and
// every offline member, whatever their roles, falls under "Offline". The
// group a row lands in matches the status its dot draws: "invisible" and
// unknown statuses draw the offline ring, an unset status draws online.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const ME = "e".repeat(64);

const member = (n: number, name: string, status: string, admin: boolean) => ({
  pseudonymKey: n === 0 ? ME : n.toString(16).padStart(64, "d"),
  displayName: name,
  roleIds: admin ? [0, 1] : [0],
  displayRole: admin ? "Admins" : "@everyone",
  status,
  timeoutUntil: null,
  lastActive: 0,
});

const MEMBERS = [
  member(0, "AdminOnline", "online", true),
  member(1, "AdminBusy", "busy", true),
  member(2, "AdminOffline", "offline", true),
  member(3, "UserOnline", "online", false),
  member(4, "UserAway", "away", false),
  member(5, "UserBusy", "busy", false),
  member(6, "UserInGame", "ingame", false),
  member(7, "UserOffline", "offline", false),
  member(8, "UserInvisible", "invisible", false),
  member(9, "UserUnset", "", false),
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
          { id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0,
            hoist: false, mentionable: false },
          { id: 1, name: "Admins", color: 0, permissions: "65536", position: 1,
            hoist: true, mentionable: false },
        ],
        myPseudonymKey: "${ME}", myRoleIds: [0, 1], removed: null,
      }];
    case "get_community_members":
      return ${JSON.stringify(MEMBERS)};
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;

test("member list groups hoisted roles, online and offline by presence", async ({ page }) => {
  await preloadMocks(page, "community", HANDLER);
  await page.goto(`/community?id=${COMMUNITY}`);
  const list = page.locator(".member-list");
  await expect(list.locator(".member-list-header")).toHaveText("Members — 10");
  await expect(list.locator(".member-group-header")).toHaveText([
    "Admins — 2",
    "Online — 5",
    "Offline — 3",
  ]);
  await expect(list.locator(".member-name")).toHaveText([
    "AdminOnline",
    "AdminBusy",
    "UserOnline",
    "UserAway",
    "UserBusy",
    "UserInGame",
    "UserUnset",
    "AdminOffline",
    "UserOffline",
    "UserInvisible",
  ]);
});
