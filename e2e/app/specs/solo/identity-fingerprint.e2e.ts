// Contract VN2 (its addendum): our own identity reads the way peers see it.
// The buddy list's identity bar and the login window's account card show the
// Host's fingerprint form (the member list's formatter), never a slice of the
// key. Ari creates an identity: the identity bar shows `get_identity`'s
// fingerprint; she logs off, and the login window's account card shows the
// same fingerprint. Neither ever shows 8+ hex of her key run together.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { LOGIN, inBuddy } from "../../../walk/nav.ts";
import { createIdentity, logOut, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

async function served(): Promise<{ publicKey: string; fingerprint: string }> {
  return (await browser.execute(async () => {
    const { invoke } = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string): Promise<{ value: { publicKey: string; fingerprint: string } }> } }).__TAURI_INTERNALS__;
    return (await invoke("get_identity")).value;
  })) as { publicKey: string; fingerprint: string };
}

/** Whether `text` shows 8 or more of `key`'s hex characters in a row. */
function showsKeySlice(text: string, key: string): boolean {
  const compact = text.replace(/\s+/g, "").toLowerCase();
  for (let i = 0; i + 8 <= key.length; i += 1) {
    if (compact.includes(key.slice(i, i + 8).toLowerCase())) return true;
  }
  return false;
}

describe("our own identity reads as peers see it: the Host's fingerprint form (VN2)", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("fingerprint"), displayName: "Ari" };

  it("shows the Host's fingerprint on the identity bar and the login card, never a key slice", async () => {
    await createIdentity(browser, me);
    await inBuddy(browser);
    const { publicKey, fingerprint } = await served();
    const bar = await shown(browser, ".identity-bar-key", NET_WAIT_MS);
    await waitUntil(async () => (await bar.getText()).trim() === fingerprint, NET_WAIT_MS, "the identity bar never showed the Host's fingerprint");
    const barText = (await bar.getText()).trim();
    console.log(`[fingerprint] identity bar: ${barText}`);
    await shot("identity-bar-fingerprint");

    await logOut(browser, me);
    await switchTo(browser, LOGIN, NET_WAIT_MS);
    const card = await shown(browser, ".account-card-key", NET_WAIT_MS);
    const cardText = (await card.getText()).trim();
    console.log(`[fingerprint] login card: ${cardText}`);
    await shot("login-card-fingerprint");

    expect({
      bar: barText === fingerprint,
      card: cardText === fingerprint,
      noSlice: !showsKeySlice(barText, publicKey) && !showsKeySlice(cardText, publicKey),
    }).toEqual({ bar: true, card: true, noSlice: true });
  });
});
