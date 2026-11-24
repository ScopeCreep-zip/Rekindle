// Group voice split (pack-grounded: Symbiosis GroupChatWindow.xml:112-165):
// in a group window, the pack's voice tab reads Call with no call, Join while
// the group's call rings, and Leave once joined. Alice and Bob (two apps,
// multiremote) and Cara (fleet) are a group of three, both apps under
// Symbiosis. Bob presses his group window's Call tab (HostGroupVoiceChat,
// which opens StartGroupCall) and ticks exactly the group's other members,
// Alice and Cara: a call whose participants match the group's roster is the
// group's call (wt/gv Inputs::group_call). Alice's group window then shows
// Join (the joinGroup op); she presses it and it shows Leave. Each state a
// shot. Starting and joining a call opens the microphone: Linux only (no
// TCC prompt may be raised on the Mac).

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, press, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { type Peer, peerAcceptGroup, startFleet, stopFleet } from "../../fleet.ts";
import { acceptRequest, createGroup, createIdentity, friendCode, friendRow, newUser, openGroup, pendingRequest, requestFriend, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";
import { befriendFleet } from "../../social.ts";

const GROUP = `Voice Squad ${Date.now() % 100000}`;

interface Tab {
  id: string;
  role: string;
  action: string;
  left: number;
  top: number;
}

/**
 * The tree nodes in `b`'s window whose own label reads `word`, drawn,
 * leftmost first; with `at`, only the one at that spot (the voice tab's
 * place: the pack may swap in a different node per page, so the place, not
 * the id, is the tab; a join-game "Join" sits elsewhere in the row).
 */
async function tabsReading(b: Browser, word: string, at?: { left: number; top: number }): Promise<Tab[]> {
  return (await b.execute(
    (w: string, spot: { left: number; top: number } | null) =>
      Array.from(document.querySelectorAll<HTMLElement>("[data-skin-node]"))
        .filter((n) => (n.querySelector(":scope > .skin-node-label .skin-node-label-text")?.textContent ?? "").trim() === w)
        .map((n) => ({ n, r: n.getBoundingClientRect() }))
        .filter(({ r }) => r.width > 0 && r.height > 0)
        .filter(({ r }) => !spot || (Math.abs(r.left - spot.left) <= 2 && Math.abs(r.top - spot.top) <= 2))
        .sort((x, y) => x.r.left - y.r.left)
        .map(({ n, r }) => ({ id: n.dataset.skinNode ?? "", role: n.dataset.skinRole ?? "", action: n.dataset.skinAction ?? "", left: Math.round(r.left), top: Math.round(r.top) })),
    word,
    at ?? null,
  )) as Tab[];
}

/** Wait until `b`'s window shows `word` (at `at`, when given: the voice tab's place); logs it as `[gv]`. */
async function voiceTab(b: Browser, who: string, word: string, at?: { left: number; top: number }): Promise<Tab> {
  let tab: Tab | undefined;
  await waitUntil(async () => (tab = (await tabsReading(b, word, at))[0]) !== undefined, NET_WAIT_MS, `${who}'s group window never showed ${word}${at ? ` at the voice tab (${at.left},${at.top})` : ""}`);
  console.log(`[gv] ${who} ${word}: ${JSON.stringify(tab)}`);
  return tab!;
}

/** Press tree node `id` in `b`'s window. */
async function pressNode(b: Browser, id: string): Promise<void> {
  await press(b, await b.$(`[data-skin-node="${id}"]`));
}

async function symbiosis(b: Browser, user: User): Promise<string> {
  await openSettings(b, "Appearance");
  const pack = builtinPacks().find((p) => p.file === "symbiosis");
  if (!pack) throw new Error("no built-in pack symbiosis");
  return applySkin(b, user, pack.name);
}

describe("a group's voice tab: Call, Join while its call rings, Leave once joined", function () {
  this.timeout(2_400_000);
  const alice = { ...newUser("alice"), displayName: "Alice" };
  const bob = { ...newUser("bob"), displayName: "Bob" };
  let peers: Peer[] = [];
  /** Where Alice's voice tab sits (read with no call). */
  let aliceVoice = { left: 0, top: 0 };

  before(async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob)]);
    const hello = `gv ${Date.now()}`;
    await requestFriend(a, alice, await friendCode(b), hello);
    await pendingRequest(b, hello);
    await acceptRequest(b, bob, hello);
    await friendRow(a, bob.displayName);
    peers = await startFleet(["Cara"]);
    await befriendFleet(a, alice, peers);
    await befriendFleet(b, bob, peers);
    await switchTo(a, BUDDY);
    await createGroup(a, alice, GROUP, [bob.displayName, "Cara"]);
    await peerAcceptGroup(peers[0], GROUP);
    await switchTo(b, BUDDY);
    await press(b, await shown(b, `[aria-label="Accept invite to ${GROUP}"]`, NET_WAIT_MS));
    await resolveIntent(b, { passphrase: bob.passphrase, appearMs: NET_WAIT_MS });
    const [ha, hb] = await Promise.all([symbiosis(a, alice), symbiosis(b, bob)]);
    await openGroup(a, GROUP);
    await skinShown(a, ha);
    await openGroup(b, GROUP);
    await skinShown(b, hb);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("with no call, Alice's group window shows Call", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const call = await voiceTab(a, "Alice", "Call");
    aliceVoice = { left: call.left, top: call.top };
    await shot("no-call-call-tab");
  });

  it("Bob calls the group from his Call tab; Alice's window shows Join", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const b = multiRemoteBrowser.getInstance("bob");
    const call = await voiceTab(b, "Bob", "Call");
    await pressNode(b, call.id);
    // StartGroupCall opens as a shell panel; find the window holding it.
    let modal = "";
    await waitUntil(
      async () => {
        for (const h of await b.getWindowHandles()) {
          await b.switchToWindow(h);
          if ((await (await b.$(".modal-title")).getText().catch(() => "")) === "Start Group Call") {
            modal = h;
            return true;
          }
        }
        return false;
      },
      NET_WAIT_MS,
      "Bob's Call tab opened no Start Group Call",
    );
    for (const name of [alice.displayName, "Cara"]) {
      await (await (await byText(b, ".start-group-call-list label.settings-option", name)).$("input[type=checkbox]")).click();
    }
    await (await b.$(".modal-container .modal-footer button:not(.form-btn-secondary)")).click();
    expect(modal).not.toBe("");
    await voiceTab(a, "Alice", "Join", aliceVoice);
    await shot("ringing-join-tab");
  });

  it("Alice presses Join and her window shows Leave", async () => {
    const a = multiRemoteBrowser.getInstance("alice");
    const join = await voiceTab(a, "Alice", "Join", aliceVoice);
    await pressNode(a, join.id);
    await voiceTab(a, "Alice", "Leave", aliceVoice);
    await shot("joined-leave-tab");
  });
});
