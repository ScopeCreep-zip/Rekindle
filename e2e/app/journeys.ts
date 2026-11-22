// Plan E7.14: journeys the real-app specs share, driven through the window
// as a user would. Built on the walk's helpers until S3 moves them here.

import { randomBytes } from "node:crypto";
import { expect } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { NETWORK_LABEL, NET_WAIT_MS, byText, doubleClick, gone, present, press, resolveIntent, shown, switchTo, typeAndSend, typeInto, untilPublished, waitForHandle, waitUntil } from "../walk/helpers.ts";
import { BUDDY, LOGIN, SETTINGS, inBuddy, menu, openAddFriend, setValue, submitJoin, underTree } from "../walk/nav.ts";

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
  // The name renders before the store hydrates ("Unknown" first, then the
  // name): the tree's name field, or the app's own bar.
  const name = (await underTree(b)) ? '[data-skin-role="nameField"] .skin-name-text' : ".identity-bar-name";
  await expect(b.$(name)).toHaveText(expect.stringContaining(user.displayName), { wait: 10_000 });
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
  // FC1: the Host answers the code once our records are held, however long
  // the first attach takes; the tab shows it by itself, nothing is pressed,
  // and says "Publishing your identity…" until then (FC3).
  await untilPublished(b, "the Public Key tab's friend code");
  let code: string;
  try {
    code = (await (await shown(b, "output.sec-friend-code-url", NET_WAIT_MS)).getText()).trim();
  } catch (err) {
    const status = await b.execute(() => document.querySelector(".add-friend-section .add-friend-code-status")?.textContent?.trim() ?? "");
    throw new Error(`the Public Key tab showed no friend code${status ? `: "${status}"` : ""} (${err instanceof Error ? err.message : String(err)})`);
  }
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
  const submit = await formSubmit(b, 'input[aria-label="Friend code"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Send Friend Request stayed disabled" });
  await submit.click();
  await resolveIntent(b, { passphrase: from.passphrase });
  // The send is held, the button disabled and the form saying "Publishing
  // your identity…", until our records are (FC3); the dialog then closes.
  await untilPublished(b, "the friend request");
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
  // A skin's tree shows its Friends view first; the app's bar has a tab.
  if (!(await underTree(b))) await (await b.$('.buddy-tab[title^="Friends"]')).click();
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

/** The buddy list's network indicator reads "Connected" (NetworkIndicator.tsx): a DHT write may land. */
async function whenConnected(b: Browser): Promise<void> {
  await inBuddy(b);
  // Under a skin's tree the status field says the network state while
  // detached (StatusField.tsx, N1) and our status ("Online") once attached.
  const tree = await underTree(b);
  await waitUntil(
    async () =>
      tree
        ? ((await (await b.$('[data-skin-role="statusText"] .skin-status-edit')).getAttribute("placeholder")) ?? "") === "Online"
        : (await (await b.$(NETWORK_LABEL)).getText()).trim() === "Connected",
    NET_WAIT_MS,
    tree ? "the skin's status field never said Online" : "the network indicator never said Connected",
  );
}

/**
 * After a submit: the U1 review (ADR 0019) appears, then is committed, or
 * the form closes with none. The review can take seconds to appear on a
 * busy node (layout run 3: the 7th community's review came after the 3 s
 * the walk allowed and stayed uncommitted), so this waits for whichever
 * comes first, up to the network bound.
 */
async function reviewThenGone(b: Browser, user: User, input: string): Promise<void> {
  let review = false;
  await waitUntil(
    async () => (review = await (await b.$(".intent-dialog")).isDisplayed().catch(() => false)) || !(await present(b, input)),
    NET_WAIT_MS,
    `neither a review nor the form closing after submit (${input})`,
  );
  if (review) await resolveIntent(b, { passphrase: user.passphrase });
  await gone(b, input, NET_WAIT_MS);
}

/**
 * The submit of the dialog form holding `input`. Another dialog can be open
 * over or under this one (the intent review is a `.modal-container` with
 * its own submit), and webdriverio's `$` refuses a selector matching two
 * (Linux, ddfc1e70b: StrictSelectorError in New Group).
 */
async function formSubmit(b: Browser, input: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  let submit: Awaited<ReturnType<Browser["$"]>> | undefined;
  await waitUntil(
    async () => {
      for (const candidate of await b.$$('.modal-container button[type="submit"]')) {
        const owns = await b.execute((btn: HTMLButtonElement, sel: string) => Boolean(btn.form?.querySelector(sel)), candidate as unknown as HTMLButtonElement, input);
        if (owns) submit = candidate;
      }
      return submit !== undefined;
    },
    NET_WAIT_MS,
    `no dialog submit whose form holds ${input}`,
  );
  if (!submit) throw new Error(`no dialog submit whose form holds ${input}`);
  return submit;
}

/** Submit the open modal's form (Create waits for "Connected", CreateCommunityModal.tsx) and resolve its review. */
async function submitModal(b: Browser, user: User, input: string): Promise<void> {
  const submit = await formSubmit(b, input);
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "the form's submit stayed disabled" });
  await submit.click();
  await reviewThenGone(b, user, input);
}

/** `user` creates community `name` from the buddy list's Communities tab, and it is listed. */
/**
 * The Communities list. Under a skin's tree, the pack's Communities control
 * (the Host places one in every tree, plan SK2-N4: `tab:communities` or
 * `knob:communities`) shows its Communities view; else the app's tab.
 */
async function communitiesTab(b: Browser): Promise<void> {
  if (await underTree(b)) {
    await press(b, await shown(b, '[data-skin-role="tab:communities"], [data-skin-role="knob:communities"]'));
    await shown(b, '[data-skin-role="view:communities"]:not([hidden])');
    return;
  }
  await (await shown(b, '.buddy-tab[title^="Communities"]')).click();
}

export async function createCommunity(b: Browser, user: User, name: string): Promise<void> {
  await whenConnected(b);
  await communitiesTab(b);
  if (await underTree(b)) {
    // A skin's tree has no BottomActionBar: the pack's first menu (the Host's registry).
    await menu(b, "Rekindle", "Create Community…");
  } else {
    // The Create Community button shows only on the Communities tab (BottomActionBar.tsx).
    await (await shown(b, '[aria-label="Create Community"]')).click();
  }
  await typeInto(await shown(b, 'input[placeholder="Community name..."]'), name);
  await submitModal(b, user, 'input[placeholder="Community name..."]');
  await byText(b, ".community-item", name.slice(0, 24), NET_WAIT_MS);
}

/** Open community `name`'s window from the buddy list (double-click) and switch to it. */
export async function openCommunity(b: Browser, name: string): Promise<void> {
  await inBuddy(b);
  await communitiesTab(b);
  const before = await b.getWindowHandles();
  await doubleClick(b, await byText(b, ".community-item", name.slice(0, 24), NET_WAIT_MS));
  await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("community-") && !before.includes(h), NET_WAIT_MS));
  await shown(b, ".community-sidebar", NET_WAIT_MS);
}

/** In the open community window, `user` creates a text channel `name`, and it is listed. */
export async function createTextChannel(b: Browser, user: User, name: string): Promise<void> {
  await (await shown(b, '[title="Create Channel"]')).click();
  await typeInto(await shown(b, 'input[placeholder="Channel name..."]'), name);
  await submitModal(b, user, 'input[placeholder="Channel name..."]');
  await byText(b, ".channel-item", name.slice(0, 24), NET_WAIT_MS);
}

/** In the open community window, select channel `name`. */
export async function selectChannel(b: Browser, name: string): Promise<void> {
  await (await byText(b, ".channel-item", name.slice(0, 24), NET_WAIT_MS)).click();
  await waitUntil(
    async () => (await (await b.$(".community-channel-header-name")).getText()).startsWith(name.slice(0, 16)),
    NET_WAIT_MS,
    `the channel header did not name ${name.slice(0, 24)}`,
  );
}

/** In the selected channel, `user` posts `text`, and it shows. */
export async function postMessage(b: Browser, user: User, text: string): Promise<void> {
  await typeAndSend(b, ".community-main .message-input-field", text);
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
  await byText(b, ".community-main .chat-message", text.slice(0, 40), NET_WAIT_MS);
}

/** `user` creates group `name` with the friends whose names start `members` (New Group, BottomActionBar). */
export async function createGroup(b: Browser, user: User, name: string, members: string[]): Promise<void> {
  await inBuddy(b);
  if (await underTree(b)) {
    await menu(b, "Rekindle", "New Group…");
  } else {
    await (await b.$('.buddy-tab[title^="Friends"]')).click();
    await (await shown(b, '[aria-label="New Group"]')).click();
  }
  await typeInto(await shown(b, "#new-group-name"), name);
  for (const m of members) await (await byText(b, "label.group-dm-picker-option", m.slice(0, 24))).click();
  await submitModal(b, user, "#new-group-name");
}

/** Open group `name`'s chat window from the buddy list (double-click its row) and switch to it. */
export async function openGroup(b: Browser, name: string): Promise<void> {
  await inBuddy(b);
  if (!(await underTree(b))) await (await b.$('.buddy-tab[title^="Friends"]')).click();
  const before = await b.getWindowHandles();
  await doubleClick(b, await byText(b, ".group-dm-row", name.slice(0, 24), NET_WAIT_MS));
  await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("chat-g") && !before.includes(h), NET_WAIT_MS));
  await shown(b, ".message-input-field", NET_WAIT_MS);
}

/** In the open community window, open Community Settings on `tab`. */
async function communitySettings(b: Browser, tab: string): Promise<void> {
  if (!(await present(b, ".modal-container .form-tabs"))) {
    await (await shown(b, '[title="Community Settings"]')).click();
    await shown(b, ".modal-container .form-tabs");
  }
  await (await byText(b, ".modal-container .form-tab", tab)).click();
}

/**
 * In the open community window, `user` creates an invite good for `uses`
 * joins (Settings, Invites; the choice's default is one use, and unlimited
 * invites are forbidden, rekindle-governance invite_quota.rs); its link.
 */
export async function createInvite(b: Browser, user: User, uses: number): Promise<string> {
  await communitySettings(b, "Invites");
  const before = (await b.$$(".invite-link-display")).length;
  await (await shown(b, ".modal-container .settings-content .form-btn-primary")).click();
  // Max uses is a choice of 1, 5, 10, 25, 50 or 100 (IJ1; InvitesTab.tsx MAX_USES_CHOICES): the smallest that covers `uses`.
  const choice = [1, 5, 10, 25, 50, 100].find((n) => n >= uses);
  if (choice === undefined) throw new Error(`an invite holds at most 100 uses, asked ${uses}`);
  await setValue(b, ".invite-create-form .invite-max-uses select", String(choice));
  await (await byText(b, ".modal-container .form-btn-primary", "Create")).click();
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 3000 });
  await waitUntil(async () => (await b.$$(".invite-link-display")).length > before, NET_WAIT_MS, "no new invite link shown");
  // The link is the read-only input's value (InvitesTab.tsx).
  const links = await b.$$(".invite-link-display input");
  const link = (await links[links.length - 1].getValue()).trim();
  if (!link.startsWith("rekindle://invite/")) throw new Error(`the new invite shows no link: "${link.slice(0, 40)}"`);
  await (await shown(b, ".modal-container .modal-close-btn")).click();
  await gone(b, ".modal-container");
  return link;
}

/**
 * `user` joins by `link` from the buddy list's Communities tab (Join
 * Community, JoinCommunityModal.tsx; a refusal fails with the Host's
 * reason, `submitJoin`), and community `name` is listed.
 */
export async function joinCommunity(b: Browser, user: User, link: string, name: string): Promise<void> {
  await whenConnected(b);
  await communitiesTab(b);
  if (await underTree(b)) await menu(b, "Rekindle", "Join Community…");
  else await (await shown(b, '[aria-label="Join Community"]')).click();
  await submitJoin(b, link, user.passphrase);
  await byText(b, ".community-item", name.slice(0, 24), NET_WAIT_MS);
}

/**
 * From now on, record every `console.warn` and `console.error` the window
 * `b` is on writes (they still print), so a spec can assert one never
 * happened; `consoleSeen` reads them. Lasts until the page reloads.
 */
export async function watchConsole(b: Browser): Promise<void> {
  await b.execute(`if (!window.__rekindleConsole) {
  window.__rekindleConsole = [];
  for (const level of ["warn", "error"]) {
    const original = console[level].bind(console);
    console[level] = (...args) => {
      window.__rekindleConsole.push(level + ": " + args.map((a) => (a instanceof Error ? a.message : typeof a === "string" ? a : JSON.stringify(a))).join(" "));
      original(...args);
    };
  }
}`);
}

/** What `watchConsole` recorded in the window `b` is on. */
export async function consoleSeen(b: Browser): Promise<string[]> {
  return (await b.execute("return window.__rekindleConsole || null;")) as string[];
}

/**
 * `user` chooses built-in skin `name` in Settings → Appearance (its radio,
 * then the intent review), and the Settings window has linked its sheet;
 * the sheet's address. Coming back to Appearance from another tab mounts
 * it afresh, so its list is awaited first; the skin already in use is not
 * clicked, so its sheet is awaited all the same.
 */
export async function applySkin(b: Browser, user: User, name: string): Promise<string> {
  await switchTo(b, SETTINGS);
  await (await byText(b, ".form-tab", "Appearance")).click();
  await shown(b, ".skin-row-shown .skin-name", NET_WAIT_MS);
  const inUse = async (): Promise<string> => (await (await b.$(".skin-row-shown .skin-name")).getText()).trim();
  if ((await inUse()) !== name) {
    let found = false;
    for (const row of await b.$$(".skin-row")) {
      if ((await (await row.$(".skin-name")).getText()).trim() !== name) continue;
      await (await row.$("input[type=radio]")).click();
      await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
      found = true;
      break;
    }
    if (!found) throw new Error(`Settings lists no skin named ${name}`);
    await waitUntil(async () => (await inUse()) === name, NET_WAIT_MS, `${name} never shown as the skin in use`);
  }
  let href = "";
  await waitUntil(
    async () => {
      href = (await b.execute('var l = document.getElementById("rekindle-skin"); return l && l.sheet ? l.getAttribute("href") : "";')) as string;
      return href.startsWith("skin://");
    },
    NET_WAIT_MS,
    "Settings never linked the skin's sheet",
  );
  return href;
}

/** In the open Settings → Appearance, `user` makes built-in skin `name` the login skin (its "Use as login skin", then the review), and its row says so. */
export async function setLoginSkin(b: Browser, user: User, name: string): Promise<void> {
  for (const row of await b.$$(".skin-row")) {
    if ((await (await row.$(".skin-name")).getText()).trim() !== name) continue;
    if (!(await (await row.$(".skin-badge")).isExisting())) {
      await press(b, await row.$(".skin-action"));
      await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
    }
    await waitUntil(async () => (await (await row.$(".skin-badge")).isExisting()), NET_WAIT_MS, `${name} never marked as the login skin`);
    return;
  }
  throw new Error(`Settings lists no skin named ${name}`);
}

/** `user` logs out from the buddy list (Logout, then the review) to the unlock screen under their name. */
export async function logOut(b: Browser, user: User): Promise<void> {
  await inBuddy(b);
  if (await underTree(b)) await menu(b, "Rekindle", "Log Off");
  else await (await b.$('[aria-label="Logout"]')).click();
  await resolveIntent(b, { passphrase: user.passphrase });
  await switchTo(b, LOGIN, NET_WAIT_MS);
  await waitUntil(
    async () => (await (await shown(b, ".lock-name", NET_WAIT_MS)).getText()).trim() === user.displayName,
    NET_WAIT_MS,
    `the unlock screen does not name ${user.displayName}`,
  );
}

/** On the unlock screen, `user` unlocks with their passphrase and the buddy list opens. */
export async function unlock(b: Browser, user: User): Promise<void> {
  await switchTo(b, LOGIN, NET_WAIT_MS);
  await typeInto(await shown(b, 'form.login-container input[type="password"]'), user.passphrase);
  const submit = await b.$('form.login-container button[type="submit"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Unlock stayed disabled (Connecting…)" });
  await submit.click();
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
  await inBuddy(b);
}
