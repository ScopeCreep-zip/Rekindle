// Contract VN2 (its addendum): our own identity reads the way peers see it.
// The buddy list and the login window's account card show the Host's
// fingerprint form (the member list's formatter), never a slice of the key.
// Ari creates an identity. Where the buddy list is the app's own layout, its
// identity bar shows `get_identity`'s fingerprint; under a skin's tree (every
// built-in pack lays one; the pack's nodes show her name, no key) the window
// shows no slice of her key. She logs off, and the login window's account
// card shows the same fingerprint (her key's first 8 hex in two groups of
// four). Nothing ever shows 8+ hex of her key run together.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { LOGIN, inBuddy, underTree } from "../../../walk/nav.ts";
import { createIdentity, logOut, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

async function served(): Promise<{ publicKey: string; fingerprint: string }> {
  return (await browser.execute(async () => {
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string): Promise<{ value: { publicKey: string; fingerprint: string } }> } }).__TAURI_INTERNALS__;
    return (await invoke("get_identity")).value;
  })) as { publicKey: string; fingerprint: string };
}

/**
 * Whether `text` shows 8 or more of `key`'s hex characters run together.
 * The fingerprint form is the key's first 8 hex in two groups of four
 * ("a1b2 c3d4"), so only a contiguous run is a key slice; whitespace is
 * not stripped.
 */
function showsKeySlice(text: string, key: string): boolean {
  const runs = text.toLowerCase().match(/[0-9a-f]{8,}/g) ?? [];
  return runs.some((run) => key.toLowerCase().includes(run.slice(0, 8)));
}

describe("our own identity reads as peers see it: the Host's fingerprint form (VN2)", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("fingerprint"), displayName: "Ari" };

  it("shows the Host's fingerprint on the identity bar and the login card, never a key slice", async () => {
    await createIdentity(browser, me);
    await inBuddy(browser);
    const { publicKey, fingerprint } = await served();
    const tree = await underTree(browser);
    let barText: string;
    if (tree) {
      // The pack's tree: its text is what the window shows of her.
      barText = ((await browser.execute(() => document.querySelector(".skin-tree")?.textContent ?? "")) as string).replace(/\s+/g, " ").trim();
      console.log(`[fingerprint] buddy list under the skin's tree (no identity bar): ${barText.slice(0, 160)}`);
    } else {
      const bar = await shown(browser, ".identity-bar-key", NET_WAIT_MS);
      await waitUntil(async () => (await bar.getText()).trim() === fingerprint, NET_WAIT_MS, "the identity bar never showed the Host's fingerprint");
      barText = (await bar.getText()).trim();
      console.log(`[fingerprint] identity bar: ${barText}`);
    }
    await shot("buddy-list-identity");

    await logOut(browser, me);
    await switchTo(browser, LOGIN, NET_WAIT_MS);
    const card = await shown(browser, ".account-card-key", NET_WAIT_MS);
    const cardText = (await card.getText()).trim();
    console.log(`[fingerprint] login card: ${cardText}`);
    await shot("login-card-fingerprint");

    expect({
      bar: tree || barText === fingerprint,
      card: cardText === fingerprint,
      noSlice: !showsKeySlice(barText, publicKey) && !showsKeySlice(cardText, publicKey),
    }).toEqual({ bar: true, card: true, noSlice: true });
  });
});
