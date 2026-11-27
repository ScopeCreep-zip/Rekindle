// Contract VN2: a group call's members are named as the Host serves them
// (participants: { publicKey, displayName }), never by a slice of a key.
// Three real apps (wdio.devices-peers.conf.ts: alice, bob, carol), since a
// daemon places no call before E4.1. Bob, friends with both, starts a group
// voice call to Alice and Carol through the Host (`start_group_call`, the
// command the Start Group Call panel sends; the call's own UI is what is
// tested here). Alice's incoming banner names Bob; she accepts, and her call
// grid names Bob and Carol by the names the Host serves, with no run of 8+
// hex anywhere in the call's UI. Accepting a call opens the microphone:
// Linux only (no TCC prompt on the Mac).

import { expect, multiRemoteBrowser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { acceptRequest, createIdentity, friendCode, friendRow, newUser, pendingRequest, requestFriend, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const app = (name: "alice" | "bob" | "carol"): Browser => multiRemoteBrowser.getInstance(name) as unknown as Browser;

async function publicKey(b: Browser): Promise<string> {
  return (await b.execute(async () => {
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string): Promise<{ value: { publicKey: string } }> } }).__TAURI_INTERNALS__;
    return (await invoke("get_identity")).value.publicKey;
  })) as string;
}

/** `from` and `to` become friends: `from` asks with `to`'s code, `to` accepts. */
async function befriend(from: Browser, fromUser: User, to: Browser, toUser: User): Promise<void> {
  const hello = `${fromUser.displayName} to ${toUser.displayName} ${Date.now()}`;
  await requestFriend(from, fromUser, await friendCode(to), hello);
  await pendingRequest(to, hello);
  await acceptRequest(to, toUser, hello);
  await friendRow(from, toUser.displayName);
}

describe("a group call names its members as the Host serves them (VN2)", function () {
  this.timeout(1_800_000);
  const alice: User = { ...newUser("gc-alice"), displayName: "Alice" };
  const bob: User = { ...newUser("gc-bob"), displayName: "Bob" };
  const carol: User = { ...newUser("gc-carol"), displayName: "Carol" };

  it("Alice's incoming banner and call grid name Bob and Carol, never by a key slice", async () => {
    const a = app("alice");
    const b = app("bob");
    const c = app("carol");
    await Promise.all([createIdentity(a, alice), createIdentity(b, bob), createIdentity(c, carol)]);
    await befriend(b, bob, a, alice);
    await befriend(b, bob, c, carol);
    const [aliceKey, carolKey] = await Promise.all([publicKey(a), publicKey(c)]);

    // Bob starts the group voice call to Alice and Carol through the Host.
    await switchTo(b, BUDDY);
    await b.execute(async (keys: string[]) => {
      const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> } }).__TAURI_INTERNALS__;
      await invoke("start_group_call", { participantPubkeys: keys, video: false, groupId: null });
    }, [aliceKey, carolKey]);

    // Alice's incoming banner names the caller as the Host serves him.
    await switchTo(a, BUDDY);
    const banner = await shown(a, ".group-call-incoming-banner", NET_WAIT_MS);
    const bannerText = (await banner.getText()).replace(/\s+/g, " ");
    console.log(`[group-call] Alice's banner: ${bannerText}`);
    await shot("group-call-incoming");

    // She accepts: her call grid names each member.
    await press(a, await byText(a, ".group-call-incoming-banner button", "Accept"));
    let names: string[] = [];
    await waitUntil(
      async () => {
        names = (await a.execute(() =>
          Array.from(document.querySelectorAll(".group-call-tile-name")).map((n) => (n.textContent ?? "").trim()),
        )) as string[];
        return names.includes("Bob") && names.includes("Carol");
      },
      NET_WAIT_MS,
      "",
    ).catch(() => {
      throw new Error(`Alice's call grid never named Bob and Carol (it shows ${JSON.stringify(names)})`);
    });
    const callText = (await a.execute(() =>
      Array.from(document.querySelectorAll(".group-call-incoming-banner, .group-call-grid, .active-call-panel"))
        .map((el) => el.textContent ?? "")
        .join(" "),
    )) as string;
    const hexRuns = callText.toLowerCase().match(/[0-9a-f]{8,}/g) ?? [];
    console.log(`[group-call] Alice's grid: ${JSON.stringify(names)}; hex runs in the call's UI: ${hexRuns.length}`);
    await shot("group-call-grid");

    expect({ caller: bannerText.includes("Bob"), names: names.includes("Bob") && names.includes("Carol"), hexRuns })
      .toEqual({ caller: true, names: true, hexRuns: [] });
  });
});
