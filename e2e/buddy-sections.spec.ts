import { test, expect } from "@playwright/test";
import { preloadMocks } from "./fixtures/mocks";

// Plan E7.2 "Buddy list": each group splits into "Online" and "Offline"
// sections (Xfire's list); an empty section is not shown. A friend's game
// shows its icon from the backend's key, or the generic glyph.

const ME = "a".repeat(64);

function friend(key: string, name: string, status: string, iconKey: string | null = null): string {
  const game = iconKey === null
    ? "null"
    : `{ gameId: 4181, gameName: "Counter-Strike 2", serverInfo: null, elapsedSeconds: 60, serverAddress: null, iconKey: "${iconKey}" }`;
  return `{
    publicKey: "${key}", displayName: "${name}", nickname: null, status: "${status}",
    statusMessage: null, gameInfo: ${game}, groupId: null, group: null,
    groupLabel: "Friends", unreadCount: 0, lastSeenAt: null,
    friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
    identityPinMismatch: false,
  }`;
}

function handler(friends: string[]): string {
  return `
  switch (cmd) {
    case "get_identity":
      return { publicKey: "${ME}", displayName: "Me" };
    case "get_friends":
      return [${friends.join(",")}];
    case "get_community_details":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;
}

test("a group lists its online friends, then its offline ones, each under its heading", async ({ page }) => {
  await preloadMocks(page, "buddy-list", handler([
    friend("b".repeat(64), "Bob", "offline"),
    friend("c".repeat(64), "Cy", "away", "xfire/8507"),
    friend("d".repeat(64), "Di", "online", "slug/no-such-icon"),
  ]));
  await page.goto("/buddy-list");
  const online = page.getByRole("group", { name: "Online" });
  const offline = page.getByRole("group", { name: "Offline" });
  await expect(online.locator(".buddy-subgroup-header")).toHaveText("Online (2)");
  await expect(online.locator(".buddy-name")).toHaveText(["Di", "Cy"]);
  await expect(offline.locator(".buddy-subgroup-header")).toHaveText("Offline (1)");
  await expect(offline.locator(".buddy-name")).toHaveText(["Bob"]);
  // The game icon: Xfire's for a mapped key, the glyph for a key with no file.
  const cy = online.locator(".buddy-item", { hasText: "Cy" });
  await expect(cy.locator("img.game-icon")).toHaveAttribute("src", /8507/);
  const di = online.locator(".buddy-item", { hasText: "Di" });
  await expect(di.locator(".game-icon-generic")).toBeVisible();
});

test("an empty section is not shown", async ({ page }) => {
  await preloadMocks(page, "buddy-list", handler([friend("b".repeat(64), "Bob", "online")]));
  await page.goto("/buddy-list");
  await expect(page.getByRole("group", { name: "Online" })).toBeVisible();
  await expect(page.getByRole("group", { name: "Offline" })).toHaveCount(0);
});
