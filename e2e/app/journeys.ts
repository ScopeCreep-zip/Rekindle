// Plan E7.14: journeys the real-app specs share, driven through the window
// as a user would. Built on the walk's helpers until S3 moves them here.

import { randomBytes } from "node:crypto";
import { expect } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, doubleClick, gone, resolveIntent, shown, switchTo, typeInto, waitForHandle } from "../walk/helpers.ts";
import { BUDDY, LOGIN, inBuddy, openAddFriend } from "../walk/nav.ts";

/** A throwaway identity: its name and passphrase live only in this run. */
export interface User {
  displayName: string;
  passphrase: string;
}

export function newUser(prefix: string): User {
  return { displayName: `${prefix}-${randomBytes(3).toString("hex")}`, passphrase: randomBytes(18).toString("base64url") };
}

/**
 * Create `user` on a first-run app: the Host makes the keys, vault and
 * database, Create waits for the Veilid node, and the buddy list opens
 * under the new name.
 */
export async function createIdentity(b: Browser, user: User): Promise<void> {
  await switchTo(b, LOGIN, NET_WAIT_MS);
  // First run opens the create form without the account picker
  // (LoginWindow.tsx); the button is there only once identities exist.
  const create = await b.$(".account-create-btn");
  if (await create.isDisplayed()) await create.click();
  await typeInto(await shown(b, 'input[placeholder="Display Name (optional)"]', NET_WAIT_MS), user.displayName);
  await typeInto(await shown(b, 'form.login-container input[type="password"]'), user.passphrase);
  const submit = await b.$('form.login-container button[type="submit"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Create Identity stayed disabled (Connecting…)" });
  await submit.click();
  await resolveIntent(b, { passphrase: user.passphrase });
  await waitForHandle(b, (h) => h === BUDDY, NET_WAIT_MS);
  await inBuddy(b);
  // The bar renders before the store hydrates ("Unknown" first, then the name).
  await expect(b.$(".identity-bar-name")).toHaveText(expect.stringContaining(user.displayName), { wait: 10_000 });
}

async function closeAddFriend(b: Browser): Promise<void> {
  await (await b.$(".modal-close-btn")).click();
  await gone(b, ".modal-container");
}

/** The friend code the Add Friend dialog's Public Key tab shows. */
export async function friendCode(b: Browser): Promise<string> {
  await inBuddy(b);
  await openAddFriend(b);
  await (await byText(b, ".form-tab", "Public Key")).click();
  const code = (await (await shown(b, "output.sec-friend-code-url", NET_WAIT_MS)).getText()).trim();
  await closeAddFriend(b);
  return code;
}

/** `from` sends a friend request with `message` to the holder of `code`. */
export async function requestFriend(b: Browser, from: User, code: string, message: string): Promise<void> {
  await inBuddy(b);
  await openAddFriend(b);
  await (await byText(b, ".form-tab", "Public Key")).click();
  await typeInto(await shown(b, 'input[aria-label="Friend code"]'), code);
  await typeInto(await shown(b, 'input[aria-label="Message (optional)"]'), message);
  await (await b.$('.modal-container form button[type="submit"]')).click();
  await resolveIntent(b, { passphrase: from.passphrase });
  await closeAddFriend(b).catch(() => undefined);
}

/** The pending request carrying `message`, once it reaches this buddy list. */
export async function pendingRequest(b: Browser, message: string): Promise<string> {
  await inBuddy(b);
  const item = await byText(b, ".pending-item", message, NET_WAIT_MS);
  return (await item.getText()).replace(/\s+/g, " ");
}

/** `user` accepts the request carrying `message`. */
export async function acceptRequest(b: Browser, user: User, message: string): Promise<void> {
  const item = await byText(b, ".pending-item", message, NET_WAIT_MS);
  await (await item.$(".pending-btn-accept")).click();
  await resolveIntent(b, { passphrase: user.passphrase });
}

/** The Friends tab's row for `name`, once listed. */
export async function friendRow(b: Browser, name: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  await inBuddy(b);
  await (await b.$('.buddy-tab[title^="Friends"]')).click();
  return byText(b, ".buddy-item", name, NET_WAIT_MS);
}

/** Open the chat window with friend `name` (double-click its row) and switch to it. */
export async function openChat(b: Browser, name: string): Promise<void> {
  const before = await b.getWindowHandles();
  await doubleClick(b, await friendRow(b, name));
  await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("chat-") && !before.includes(h), NET_WAIT_MS));
  await shown(b, ".message-input-field", NET_WAIT_MS);
}

/** `who`'s open chat window shows a message containing `text`. */
export async function seesMessage(b: Browser, who: User, text: string): Promise<void> {
  try {
    await byText(b, ".chat-message", text, NET_WAIT_MS);
  } catch (err) {
    throw new Error(`${who.displayName}'s chat window: ${err instanceof Error ? err.message : String(err)}`);
  }
}
