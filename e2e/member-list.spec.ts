import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// The community member list renders the groups the Host decided
// (`CommunityMemberView.group`; the rules are tested in rekindle-types
// `dto::member_groups`): a header wherever the group changes, in the Host's
// order, with the group's count. The list does no grouping of its own, so a
// row the Host put under "Online" stays there whatever status its dot draws.

const COMMUNITY = `VLD0:${"g".repeat(43)}`;
const ME = "e".repeat(64);

type Group = { kind: "role"; id: number; name: string } | { kind: "online" } | { kind: "offline" };
const ADMINS: Group = { kind: "role", id: 1, name: "Admins" };
const ONLINE: Group = { kind: "online" };
const OFFLINE: Group = { kind: "offline" };

const member = (n: number, name: string, status: string, admin: boolean, group: Group) => ({
  pseudonymKey: n === 0 ? ME : n.toString(16).padStart(64, "d"),
  displayName: name,
  roleIds: admin ? [0, 1] : [0],
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
