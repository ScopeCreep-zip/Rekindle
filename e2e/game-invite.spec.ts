import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { preloadMocks, clearMocks } from "./fixtures/mocks";
import { openWindow } from "./fixtures/paint";

// Plan E7.2 "Rich presence and join game": a friend's `GameInvite` shows in
// their chat window and Join is `launch_game_to_server` (LaunchGameToServer,
// Routine); the invite button offers only the favourites the Host marks
// `invitable`; Settings → Application keeps the favourites.

const GOLDEN = JSON.parse(
  readFileSync(
    fileURLToPath(
      new URL("../crates/rekindle-types/tests/golden/subscription_events.json", import.meta.url),
    ),
    "utf8",
  ),
) as Record<string, unknown>;

const BOB = "b".repeat(64);

const FAVOURITES = `[
  { gameId: 4181, gameName: "Counter-Strike 2", iconKey: null, address: "203.0.113.7:27015", label: "Scrims", addedAt: 1, invitable: true },
  { gameId: 4900, gameName: "Valorant", iconKey: null, address: "198.51.100.2:7777", label: "Hidden", addedAt: 2, invitable: false },
]`;

const HANDLER = `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "get_identity":
      return { publicKey: "${"a".repeat(64)}", displayName: "Alice" };
    case "get_friends":
      return [{
        publicKey: "${BOB}", displayName: "Bob", nickname: null, status: "online",
        statusMessage: null, gameInfo: null, group: null, unreadCount: 0,
        lastSeenAt: null, friendshipState: "accepted", handshakeError: null, handshakeRetrying: false,
      }];
    case "get_favourite_servers":
      return ${FAVOURITES};
    case "add_favourite_server":
      return ${FAVOURITES};
    case "get_game_totals":
      return [{ gameId: 4181, gameName: "Counter-Strike 2", iconKey: null, totalSecs: 60, lastPlayed: 1 }];
    case "send_game_invite":
      return "outbox-1";
    case "get_message_history":
    case "get_pending_requests":
    case "get_missed_calls":
      return [];
    default:
      return null;
  }
`;

async function calls(page: Page, cmd: string): Promise<Record<string, unknown>[]> {
  return page.evaluate(
    (cmd) =>
      ((window as any).__ipcCalls as { cmd: string; args: Record<string, unknown> }[])
        .filter((c) => c.cmd === cmd)
        .map((c) => c.args),
    cmd,
  );
}

test.afterEach(async ({ page }) => {
  await clearMocks(page);
});

test("a friend's invite shows host:port and Join launches the game", async ({ page }) => {
  await preloadMocks(page, `chat-${BOB.slice(0, 16)}`, HANDLER);
  await openWindow(page, `/chat?peer=${BOB}`);
  await page.waitForFunction(() => typeof (window as any).__eventChannelId === "number");
  const payload = GOLDEN["friend.gameInvite"];
  expect(payload).toBeDefined();
  await page.evaluate((envelope) => {
    const w = window as any;
    w.__TAURI_INTERNALS__.runCallback(w.__eventChannelId, { index: 0, message: envelope });
  }, { cursor: null, channel: "chat-event", payload });
  const invites = page.getByRole("list", { name: "Game invites from Bob" });
  await expect(invites).toContainText("203.0.113.7:27015");
  await invites.getByRole("button", { name: "Join" }).click();
  await expect.poll(() => calls(page, "launch_game_to_server")).toEqual([
    { gameId: 4181, serverAddress: "203.0.113.7:27015" },
  ]);
  await expect(invites).toHaveCount(0);
});

test("the invite button offers only invitable favourites", async ({ page }) => {
  await preloadMocks(page, `chat-${BOB.slice(0, 16)}`, HANDLER);
  await openWindow(page, `/chat?peer=${BOB}`);
  await page.getByRole("button", { name: "Invite Bob to a game server" }).click();
  const offered = page.getByRole("list", { name: "Invite Bob to a server" }).getByRole("listitem");
  await expect(offered).toHaveCount(1);
  await expect(offered).toContainText("Scrims");
  await offered.getByRole("button", { name: "Invite" }).click();
  await expect.poll(() => calls(page, "send_game_invite")).toEqual([
    { publicKey: BOB, gameId: 4181, server: "203.0.113.7:27015" },
  ]);
  await expect(page.getByText("Invited Bob to Counter-Strike 2 at 203.0.113.7:27015.")).toBeVisible();
});

test("Settings → Application adds a favourite server", async ({ page }) => {
  await preloadMocks(page, "settings", HANDLER);
  await openWindow(page, "/settings?tab=application");
  await expect(page.getByText("Scrims (Counter-Strike 2, 203.0.113.7:27015)")).toBeVisible();
  await page.getByRole("combobox", { name: "Game" }).selectOption("4181");
  await page.getByRole("textbox", { name: "Server address (host:port)" }).fill("192.0.2.9:27016");
  await page.getByRole("textbox", { name: "Label" }).fill("LAN");
  await page.getByRole("button", { name: "Add" }).click();
  await expect.poll(() => calls(page, "add_favourite_server")).toEqual([
    { gameId: 4181, server: "192.0.2.9:27016", label: "LAN" },
  ]);
});
