// Plan E7.2 G6: a group's game lobby across two apps (multiremote,
// wdio.peers.conf.ts) and a fleet peer (a group holds three or more). Alice
// is truly shown in a game: a copy of /bin/sleep runs under a name the game
// DB lists for Linux (Counter-Strike 2's `cs2`, default_games.json; the
// scanner resolves processes by name, scanner.rs resolve_entry), with "Detect
// running games" and "Show friends the game I'm playing" on. Alice hosts from
// the group window; Bob's card offers Join; Bob joins; Alice launches on a
// named server through the Host's review; Bob's card then offers the game,
// and his one press goes through the intent dialog naming the server, which
// he cancels (no game starts on his side). Nothing launches by itself. Each
// asserted state is a shot. Linux only: the stand-in game is a Linux process
// name, and macOS's scanner reads other names.

import { type ChildProcess, spawn } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, byText, resolveIntent, shown, waitUntil } from "../../../walk/helpers.ts";
import { closeSettings, openSettings, optionBox } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, startFleet, stopFleet } from "../../fleet.ts";
import {
  acceptRequest,
  createGroup,
  createIdentity,
  friendCode,
  friendRow,
  newUser,
  openGroup,
  pendingRequest,
  requestFriend,
} from "../../journeys.ts";
import { shot } from "../../shots.ts";
import { befriendFleet } from "../../social.ts";

const GAME = { id: 4181, name: "Counter-Strike 2", process: "cs2" };
const GROUP = "Lobby Night";
const SERVER = "192.0.2.10:27015";

/** Tick a Settings checkbox on (`tab`, `text`), through its review if it asks one. */
async function turnOn(b: Browser, passphrase: string, tab: string, text: string): Promise<void> {
  await openSettings(b, tab);
  const box = await optionBox(b, text);
  await box.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: `"${text}" stayed disabled` });
  if (!(await box.isSelected())) {
    await box.click();
    await resolveIntent(b, { passphrase, appearMs: 1500 });
    await waitUntil(async () => (await optionBox(b, text)).isSelected(), WAIT_MS, `"${text}" did not turn on`);
  }
  await closeSettings(b);
}

/** The card's button `text` in `b`'s group window, and the Host's refusal when it is disabled ("" when enabled). */
async function lobbyButton(b: Browser, text: string): Promise<string> {
  const button = await byText(b, ".game-lobby-actions button", text, NET_WAIT_MS);
  return (await button.getAttribute("aria-disabled")) === "true" ? ((await button.getAttribute("title")) ?? "?") : "";
}

/** `b`'s card lists the members named `names`, in any order. */
async function membersShown(b: Browser, names: string[]): Promise<void> {
  await waitUntil(
    async () => {
      const shownNames = (await b.execute(() =>
        Array.from(document.querySelectorAll(".game-lobby-member")).map((e) => (e.textContent ?? "").trim()),
      )) as string[];
      return names.every((n) => shownNames.includes(n)) && shownNames.length === names.length;
    },
    NET_WAIT_MS,
    `the lobby never listed exactly ${names.join(", ")}`,
  );
}

describe("a group's game lobby: host, join, launch through the review", function () {
  this.timeout(1_800_000);
  const alice = newUser("alice");
  const bob = newUser("bob");
  let peers: Peer[] = [];
  let game: ChildProcess | undefined;
  let gameDir = "";

  before(function () {
    if (process.platform !== "linux") throw new Error("Linux only: the stand-in game is a Linux process name the scanner resolves");
  });

  after(async () => {
    game?.kill();
    if (gameDir) rmSync(gameDir, { recursive: true, force: true });
    await stopFleet(peers);
  });

  it("Alice and Bob are friends, and both are in a group with a fleet peer", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    const hello = `lobby friends ${Date.now()}`;
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, bob, hello);
    await friendRow(a, bob.displayName);
    peers = await startFleet(["Cara"]);
    await befriendFleet(a, alice, peers);
    await createGroup(a, alice, GROUP, [bob.displayName, "Cara"]);
    await (await shown(b, `button[aria-label="Accept invite to ${GROUP}"]`, NET_WAIT_MS)).click();
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: 1500 });
    await peerAcceptGroup(peers[0], GROUP);
    await Promise.all([openGroup(a, GROUP), openGroup(b, GROUP)]);
  });

  it("Alice is shown in a game: a process the game DB names runs, detection and showing on", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    gameDir = mkdtempSync(path.join(tmpdir(), "rekindle-game-"));
    const exe = path.join(gameDir, GAME.process);
    copyFileSync("/bin/sleep", exe);
    game = spawn(exe, ["3600"], { stdio: "ignore" });
    await turnOn(a, alice.passphrase, "Application", "Detect running games");
    await turnOn(a, alice.passphrase, "Privacy", "Show friends the game I'm playing");
    // Shown, not only detected: Bob's buddy list names Alice's game.
    await waitUntil(
      async () =>
        (await b.execute(
          (who: string, g: string) =>
            Array.from(document.querySelectorAll(".buddy-item")).some((r) => (r.textContent ?? "").includes(who) && (r.textContent ?? "").includes(g)),
          alice.displayName,
          GAME.name,
        )) as boolean,
      NET_WAIT_MS * 2,
      `Bob never saw Alice playing ${GAME.name}`,
    );
  });

  it("Alice hosts from the group window: her card refuses Join and Leave, and Launch until someone joins", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    await (await byText(a, ".game-lobby button", "Host a game lobby", NET_WAIT_MS)).click();
    await shown(a, ".game-lobby-title", NET_WAIT_MS);
    await waitUntil(async () => (await (await a.$(".game-lobby-game")).getText()).includes(GAME.name), NET_WAIT_MS, "the card never named the game");
    expect(await lobbyButton(a, "Join")).toBe("You host this lobby");
    expect(await lobbyButton(a, "Launch")).toBe("No one else has joined yet");
    expect(await lobbyButton(a, "Stop Hosting")).toBe("");
    await shot("host-card");
  });

  it("Bob's card offers Join, the others refused with the Host's reasons", async () => {
    const b = multiRemoteBrowser.getInstance("bob");
    await shown(b, ".game-lobby-title", NET_WAIT_MS);
    expect(await lobbyButton(b, "Join")).toBe("");
    expect(await lobbyButton(b, "Launch")).toBe("Only the host can do this");
    expect(await lobbyButton(b, "Stop Hosting")).toBe("Only the host can do this");
    await shot("member-card");
  });

  it("Bob joins: both cards list both, and Alice's Launch is allowed", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await (await byText(b, ".game-lobby-actions button", "Join")).click();
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: 1500 });
    const names = (forYou: boolean): string[] => (forYou ? ["You", bob.displayName] : [alice.displayName, "You"]);
    await membersShown(a, names(true));
    await membersShown(b, names(false));
    await waitUntil(async () => (await lobbyButton(a, "Launch")) === "", NET_WAIT_MS, "Alice's Launch stayed refused after Bob joined");
    expect(await lobbyButton(b, "Leave")).toBe("");
    await shot("joined");
  });

  it("Alice launches on a named server through the Host's review, which names it", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const field = await shown(a, ".game-lobby-server");
    await field.setValue(SERVER);
    await (await byText(a, ".game-lobby-actions button", "Launch")).click();
    const review = await shown(a, ".intent-dialog", NET_WAIT_MS);
    await waitUntil(async () => (await review.getText()).includes(SERVER), WAIT_MS, `the launch review did not name ${SERVER}`);
    await shot("launch-review");
    await resolveIntent(a, { passphrase: alice.passphrase, appearMs: 0 });
  });

  it("Bob's card offers the game on that server, and nothing launched by itself", async () => {
    const b = multiRemoteBrowser.getInstance("bob");
    const launched = await shown(b, ".game-lobby-launched", NET_WAIT_MS);
    await waitUntil(async () => (await launched.getText()).includes(SERVER), NET_WAIT_MS, `Bob's card never showed the game at ${SERVER}`);
    expect(await (await b.$(".intent-dialog")).isExisting()).toBe(false);
    await shot("launched");
  });

  it("Bob's one press goes through the intent dialog naming the server, and he cancels it", async () => {
    const b = multiRemoteBrowser.getInstance("bob");
    await (await byText(b, ".game-lobby-launched button", "Join the game")).click();
    const review = await shown(b, ".intent-dialog", NET_WAIT_MS);
    await waitUntil(async () => (await review.getText()).includes(SERVER), WAIT_MS, `Bob's launch review did not name ${SERVER}`);
    await shot("member-launch-review");
    expect(await resolveIntent(b, { passphrase: bob.passphrase, cancel: true, appearMs: 0 })).toBe("cancelled");
  });
});
