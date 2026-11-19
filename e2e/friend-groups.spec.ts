import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Renaming a friend group renames that group by its id (the Host's
// `FriendGroupRename { group_id }`). The header once asked
// `create_friend_group(oldName)` for an id, which inserts a new row (or
// fails on the owner's unique name) and never renamed anything (walk W2).

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CY = "c".repeat(64);

function friend(key: string, name: string, groupId: number | null, group: string | null): string {
  return `{
    publicKey: "${key}", displayName: "${name}", nickname: null, status: "online",
    statusMessage: null, gameInfo: null, groupId: ${groupId}, group: ${group ? `"${group}"` : null},
    groupLabel: "${group ?? "Friends"}", unreadCount: 0, lastSeenAt: null,
    friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
    identityPinMismatch: false,
  }`;
}

const HANDLER = `
  switch (cmd) {
    case "get_identity":
      return { publicKey: "${ALICE}", displayName: "Alice" };
    case "get_friends":
      return [${friend(BOB, "Bob", 7, "Raid")}, ${friend(CY, "Cy", null, null)}];
    case "create_friend_group":
      return { groupId: 99 };
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

test("renaming a group renames it by its id and never creates one", async ({ page }) => {
  await preloadMocks(page, "buddy-list", HANDLER);
  await openWindow(page, "/buddy-list");

  const raid = page.locator(".buddy-group-header", { hasText: "Raid" });
  await expect(raid).toBeVisible({ timeout: 10_000 });
  await raid.click({ button: "right" });
  const input = page.locator(".group-create-input");
  await expect(input).toHaveValue("Raid");
  await input.fill("Clan");
  await input.press("Enter");

  await expect
    .poll(() =>
      page.evaluate(() =>
        ((window as any).__ipcCalls ?? []).filter((c: { cmd: string }) => c.cmd === "rename_friend_group"),
      ),
    )
    .toEqual([{ cmd: "rename_friend_group", args: { groupId: 7, name: "Clan" } }]);
  const creates = await page.evaluate(() =>
    ((window as any).__ipcCalls ?? []).filter((c: { cmd: string }) => c.cmd === "create_friend_group"),
  );
  expect(creates).toEqual([]);

  // The default group is not ours: it offers no rename.
  await page.locator(".buddy-group-header", { hasText: "Friends" }).click({ button: "right" });
  await expect(page.locator(".group-create-input")).toHaveCount(0);
});
