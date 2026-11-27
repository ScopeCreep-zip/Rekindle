// Plan E7.14: journeys the real-app specs share, driven through the window
// as a user would. Built on the walk's helpers until S3 moves them here.

import { expect } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { CHANNEL_COMPOSER, NET_WAIT_MS, byText, doubleClick, gone, present, press, resolveIntent, shown, switchTo, typeAndSend, typeInto, untilPublished, communityWindowLabel, waitForHandle, waitUntil } from "../walk/helpers.ts";
import { BUDDY, LOGIN, closeModal, closeSettings, inBuddy, menu, openAddFriend, openSettings, setValue, submitJoin, underTree } from "../walk/nav.ts";
import { showSkinView, skinViewShown } from "./journeys_skins.ts";
import type { User } from "./user.ts";

export { type User, newUser } from "./user.ts";
export * from "./journeys_skins.ts";

/**
 * Create `user` on a first-run app: the Host makes the keys, vault and
 * database, Create waits for the Veilid node, and the buddy list opens
 * under the new name. Auto-away is then turned off unless `keepAutoAway`.
 */
export async function createIdentity(b: Browser, user: User, opts: { keepAutoAway?: boolean } = {}): Promise<void> {
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
  // The auto-away spec (AA1) keeps it on, to watch it fire and be turned off.
  if (!opts.keepAutoAway) await autoAwayOff(b, user);
}

/** Settings > Application's auto-away select. */
const AUTO_AWAY = '//div[contains(@class,"form-field")][.//label[contains(.,"Go away after inactivity")]]//select';

/**
 * Turn `user`'s auto-away off (Settings > Application > "Disabled", the
 * stored 0 the Host's idle stage reads as off: host/idle.rs). Idle is the
 * OS's input idle, which WebDriver input does not reset, so with nobody at
 * the machine every identity would go Away after the default 10 minutes
 * and presence checks would race it (lead, PR1 trace). Read back after
 * reopening Settings, so the stored value is what holds.
 */
export async function autoAwayOff(b: Browser, user: User): Promise<void> {
  await openSettings(b, "Application");
  await setValue(b, AUTO_AWAY, "0");
  // Under load the review can come late, and an unanswered one drops the
  // change (it read back "10 minutes"); the read-back below proves it.
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 10_000 });
  await closeSettings(b);
  await openSettings(b, "Application");
  await waitUntil(
    async () => (await (await b.$(AUTO_AWAY)).getValue()) === "0",
    NET_WAIT_MS,
    `${user.displayName}'s auto-away did not read back Disabled`,
  );
  await closeSettings(b);
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
  // An accept is always reviewed (trust-changing, with its acknowledgement):
  // wait for the review and require it committed. The 3 s default let a
  // review that came up later return "none" and stay open (r27 friend-dm,
  // Bob's Accept review unticked, Alice never listed).
  expect(await resolveIntent(b, { passphrase: user.passphrase, appearMs: NET_WAIT_MS })).toBe("committed");
}

/**
 * The Friends list on screen: under a skin's tree, its Friends view (the
 * tree may still show the Communities view a community journey left it
 * on); otherwise the app's Friends tab.
 */
export async function friendsView(b: Browser): Promise<void> {
  if (await underTree(b)) {
    if (!(await skinViewShown(b, "friends"))) await showSkinView(b, "friends");
  } else {
    await (await b.$('.buddy-tab[title^="Friends"]')).click();
  }
}

/** The Communities list on screen: a tree's Communities view, else the app's Communities tab. */
export async function communitiesView(b: Browser): Promise<void> {
  if (await underTree(b)) {
    if (!(await skinViewShown(b, "communities"))) await showSkinView(b, "communities");
  } else {
    await (await b.$('.buddy-tab[title^="Communities"]')).click();
  }
}

/**
 * The Friends tab's row for friend `name`, once listed as a friend: a buddy
 * row outside the pending-request group (BuddyGroup `.buddy-group-pending`,
 * "Awaiting Response"), which lists a request the other side has not yet
 * accepted. Matching any row with the name passed while the request was
 * still pending (Linux shots, 2026-11-23).
 */
export async function friendRow(b: Browser, name: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  await inBuddy(b);
  await friendsView(b);
  let found: Awaited<ReturnType<Browser["$"]>> | undefined;
  await waitUntil(
    async () => {
      for (const row of await b.$$(".buddy-item")) {
        if (!(await row.isDisplayed().catch(() => false))) continue;
        if (!(await row.getText().catch(() => "")).includes(name)) continue;
        const pending = (await b.execute((el: HTMLElement) => el.closest(".buddy-group-pending") !== null, row as unknown as HTMLElement)) as boolean;
        if (pending) continue;
        found = row;
        return true;
      }
      return false;
    },
    NET_WAIT_MS,
    `${name} was never listed as a friend (outside Awaiting Response)`,
  );
  if (!found) throw new Error(`${name}: no friend row`);
  return found;
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

/**
 * The Host accepts writes: a DHT write may land. Read from the Host's own
 * network state (`get_network_status`: attached, public internet ready, and
 * its lifecycle's `canWrite`), never inferred from text a window draws (our
 * presence can read Away or Appear Offline while attached or detached).
 */
async function whenConnected(b: Browser): Promise<void> {
  await inBuddy(b);
  let last = "";
  await waitUntil(
    async () => {
      const s = (await b.execute(async () => {
        const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: (cmd: string) => Promise<unknown> } };
        return (await w.__TAURI_INTERNALS__!.invoke("get_network_status")) as { isAttached: boolean; publicInternetReady: boolean; canWrite: boolean };
      })) as { isAttached: boolean; publicInternetReady: boolean; canWrite: boolean };
      last = JSON.stringify(s);
      return s.isAttached && s.publicInternetReady && s.canWrite;
    },
    NET_WAIT_MS,
    "",
  ).catch(() => {
    throw new Error(`the Host never accepted writes (network status: ${last})`);
  });
}

/**
 * The settled state a window's shot claims (lead, default-windows): the Host
 * accepts writes (as `whenConnected`) and our general route is available, so
 * the status field reads no "No Route" (`network.store.ts` networkLabel).
 * Read from `get_network_status`, never from the drawn words.
 */
export async function whenSettled(b: Browser): Promise<void> {
  await inBuddy(b);
  let last = "";
  await waitUntil(
    async () => {
      const s = (await b.execute(async () => {
        const w = window as unknown as { __TAURI_INTERNALS__?: { invoke: (cmd: string) => Promise<unknown> } };
        return (await w.__TAURI_INTERNALS__!.invoke("get_network_status")) as { isAttached: boolean; publicInternetReady: boolean; canWrite: boolean; generalRoute: string };
      })) as { isAttached: boolean; publicInternetReady: boolean; canWrite: boolean; generalRoute: string };
      last = JSON.stringify(s);
      return s.isAttached && s.publicInternetReady && s.canWrite && s.generalRoute === "available";
    },
    NET_WAIT_MS,
    "",
  ).catch(() => {
    throw new Error(`the Host never settled with a general route (network status: ${last})`);
  });
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
export async function communitiesTab(b: Browser): Promise<void> {
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
  const label = await communityWindowLabel(b, name);
  await communitiesTab(b);
  await doubleClick(b, await byText(b, ".community-item", name.slice(0, 24), NET_WAIT_MS));
  await switchTo(b, label, NET_WAIT_MS);
  await shown(b, ".community-sidebar", NET_WAIT_MS);
}

/** How many channel rows in the open community window show `name`. */
export async function channelCount(b: Browser, name: string): Promise<number> {
  return (await b.execute(
    (n: string) => Array.from(document.querySelectorAll(".channel-item")).filter((r) => (r.textContent ?? "").includes(n)).length,
    name.slice(0, 24),
  )) as number;
}

/**
 * In the open community window, `user` creates a text channel `name`, and it
 * is listed: one more row with its name than before (a match alone passed on
 * the genesis #general when a spec created a second "general").
 */
export async function createTextChannel(b: Browser, user: User, name: string): Promise<void> {
  const before = await channelCount(b, name);
  await (await shown(b, '[title="Create Channel"]')).click();
  await typeInto(await shown(b, 'input[placeholder="Channel name..."]'), name);
  await submitModal(b, user, 'input[placeholder="Channel name..."]');
  await waitUntil(async () => (await channelCount(b, name)) === before + 1, NET_WAIT_MS, `#${name.slice(0, 24)} was never added (still ${before} rows)`);
}

/**
 * In the open community window, `user` sets their name in this community
 * (Community Settings, Overview, MyNameSection: SetCommunityDisplayName), and
 * the section says members see that name.
 */
export async function setCommunityName(b: Browser, user: User, name: string): Promise<void> {
  await communitySettings(b, "Overview");
  const input = await shown(b, ".my-name-section input.form-input");
  await input.clearValue();
  await typeInto(input, name);
  await (await shown(b, ".my-name-section button[type=submit]")).click();
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
  await waitUntil(
    async () => ((await (await b.$("#my-name-hint")).getText().catch(() => "")) ?? "").includes(name),
    NET_WAIT_MS,
    `the community name never became ${name}`,
  );
  await closeModal(b);
}

/**
 * In the open community window, `user` sets whether the community sees the
 * channel they're in (Community Settings, Privacy: "Share which channel
 * you're in"; default-deny, PresenceSharingPolicy share_location), and the
 * choice reads back.
 */
export async function setLocationSharing(b: Browser, user: User, share: boolean): Promise<void> {
  await communitySettings(b, "Privacy");
  // The location choice's own field: Privacy holds three selects (location,
  // activity, last seen), and an ambiguous selector fails WebdriverIO's
  // strict mode.
  const select = '//div[contains(@class,"modal-container")]//div[contains(@class,"form-field")][.//label[contains(.,"Share which channel")]]//select';
  const value = share ? "members" : "none";
  await (await shown(b, select)).waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "the location choice stayed disabled" });
  if ((await (await b.$(select)).getValue()) !== value) {
    await setValue(b, select, value);
    // A presence-sharing change is reviewed (IntentKind::SetPresencePolicy,
    // trust-changing): wait for its review and require it committed; the
    // select stays disabled until the Host answers (PrivacyTab saving), and
    // a 1.5 s wait left the review unanswered (Linux, c18558272).
    expect(await resolveIntent(b, { passphrase: user.passphrase, appearMs: NET_WAIT_MS })).toBe("committed");
  }
  await waitUntil(
    async () => (await (await b.$(select)).getValue()) === value && (await (await b.$(select)).isEnabled()),
    NET_WAIT_MS,
    `location sharing never read back ${value}`,
  );
  await closeModal(b);
}

/** A status as one's own status line names it (StatusPicker's labels), as a
 *  value; "Appear Offline" is what others see as offline. */
const OWN_STATUS: Record<string, string> = { Online: "online", Away: "away", Busy: "busy", "Appear Offline": "offline" };

/** A status dot's accessible name (StatusDot.tsx "Status: <label>"), as the same value. */
export const DOT_STATUS: Record<string, string> = {
  "Status: Online": "online",
  "Status: Away": "away",
  "Status: Do not disturb": "busy",
  "Status: In game": "ingame",
  "Status: Offline": "offline",
  "Status: Appear offline": "offline",
};

/**
 * `b`'s own effective status as its buddy list's status line shows it (the
 * app's StatusPicker, or a skin tree's status field), as a status value;
 * back on the window `b` was on.
 */
export async function ownStatusValue(b: Browser): Promise<string> {
  const here = await b.getWindowHandle();
  await switchTo(b, BUDDY);
  const label = (await b.execute(
    () =>
      document.querySelector(".status-picker .buddy-name")?.textContent?.trim() ||
      document.querySelector<HTMLInputElement>(".skin-status-edit")?.placeholder?.trim() ||
      "",
  )) as string;
  await b.switchToWindow(here);
  return OWN_STATUS[label] ?? `unread "${label}"`;
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
  await typeAndSend(b, CHANNEL_COMPOSER, text);
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
  for (const m of members) {
    // Wait on what decides: the Host's eligibility for this friend (its
    // groupDmBlock; the box is disabled with the Host's reason until it is
    // null), not on time. A friend whose session is still opening is not
    // eligible yet.
    let state = { found: false, disabled: true, reason: "" };
    await waitUntil(
      async () => {
        state = (await b.execute((n: string) => {
          const opt = Array.from(document.querySelectorAll("label.group-dm-picker-option")).find((l) => (l.querySelector(".buddy-name")?.textContent ?? "").includes(n));
          const box = opt?.querySelector<HTMLInputElement>("input[type=checkbox]");
          return { found: !!box, disabled: box?.disabled ?? true, reason: (opt?.querySelector(".group-dm-picker-reason")?.textContent ?? "").trim() };
        }, m.slice(0, 24))) as typeof state;
        return state.found && !state.disabled;
      },
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(state.found ? `${m} never became eligible for the group (the Host says: ${state.reason || "no reason"})` : `${m} is not in the New Group picker`);
    });
    await (await byText(b, "label.group-dm-picker-option", m.slice(0, 24))).click();
    await waitUntil(
      async () => (await b.execute((n: string) => Array.from(document.querySelectorAll("label.group-dm-picker-option")).find((l) => (l.querySelector(".buddy-name")?.textContent ?? "").includes(n))?.querySelector<HTMLInputElement>("input[type=checkbox]")?.checked ?? false, m.slice(0, 24))) as boolean,
      NET_WAIT_MS,
      `${m} was not ticked in the New Group picker`,
    );
  }
  await submitModal(b, user, "#new-group-name");
}

/** Open group `name`'s chat window from the buddy list (double-click its row) and switch to it. */
export async function openGroup(b: Browser, name: string): Promise<void> {
  await inBuddy(b);
  await friendsView(b);
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
