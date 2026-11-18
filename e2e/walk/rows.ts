// Plan E7.13: the walked rows, from
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md.
// Ordered so ONE throwaway identity carries the whole walk: create it, walk
// the buddy list, log out, walk the picker and lock screen back in, Quit,
// and delete it last. Selectors come from the cited components; where a
// control has no stable attribute the note on its row says so.

import type { Browser } from "webdriverio";
import type { App } from "./launch.ts";
import {
  Key, MOD_KEY, NET_WAIT_MS, Skip, WAIT_MS, byText, doubleClick, gone, identityBarState,
  present, press, resolveIntent, shown, sleep, switchTo, typeInto, waitForHandle, waitUntil, within,
} from "./helpers.ts";
import { BUDDY, LOGIN, SETTINGS, closeModal, inBuddy, menu, openAddFriend } from "./nav.ts";
import { pageLog, recordPage } from "./probe.ts";

export interface WalkState {
  displayName: string;
  passphrase: string;
  communityName: string;
}

export interface RowContext {
  b: Browser;
  app: App;
  s: WalkState;
  /** Take the row's screenshot now (before a window it shows closes). */
  shot(): Promise<void>;
}

export interface Row {
  id: string;
  title: string;
  run(ctx: RowContext): Promise<string | void>;
}

function settingsRow(id: string, item: string): Row {
  return {
    id,
    title: `Menu > Settings > ${item}`,
    async run({ b, shot }) {
      await inBuddy(b);
      await menu(b, "Rekindle", item);
      await switchTo(b, SETTINGS);
      await shown(b, "body");
      await sleep(500);
      await shot();
      await b.closeWindow();
      await inBuddy(b);
      return `settings window opened on ${item} (tab not asserted: no stable tab selector read)`;
    },
  };
}

/**
 * After logout the login window opens on the last account's unlock screen
 * (`.lock-name`, "← Switch Account", LoginWindow.tsx:211-212), not on the
 * picker: wait for either, then go back to the picker.
 */
async function toPicker(b: Browser): Promise<void> {
  await switchTo(b, LOGIN, NET_WAIT_MS);
  await waitUntil(
    async () => (await present(b, ".account-bubble")) || (await present(b, ".lock-name")),
    NET_WAIT_MS,
    "login window showed neither the picker nor the unlock screen",
  );
  if (!(await present(b, ".account-bubble"))) {
    await (await byText(b, "button.form-btn-secondary", "Switch Account")).click();
    await shown(b, ".account-bubble");
  }
}

/** Create `s`'s identity from the login window and land on the buddy list. */
export async function createIdentity(b: Browser, s: WalkState): Promise<void> {
  await switchTo(b, LOGIN, NET_WAIT_MS);
  if (await present(b, ".account-create-btn")) await (await b.$(".account-create-btn")).click();
  await typeInto(await shown(b, 'input[placeholder="Display Name (optional)"]', NET_WAIT_MS), s.displayName);
  await typeInto(await shown(b, 'form.login-container input[type="password"]'), s.passphrase);
  const submit = await b.$('form.login-container button[type="submit"]');
  await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Create Identity stayed disabled (Connecting…)" });
  await submit.click();
  await resolveIntent(b, { passphrase: s.passphrase });
  await waitForHandle(b, (h) => h === BUDDY, NET_WAIT_MS);
  await inBuddy(b);
  // The bar renders before the store hydrates ("Unknown" first, then the name).
  let name = "";
  await waitUntil(
    async () => (name = await (await shown(b, ".identity-bar-name")).getText()).includes(s.displayName),
    WAIT_MS,
    "",
  ).catch(() => {
    throw new Error(`identity bar shows "${name}" ${WAIT_MS} ms after the buddy list opened`);
  });
}

async function generateInvite(ctx: RowContext): Promise<void> {
  const { b, s } = ctx;
  await openAddFriend(b);
  await (await byText(b, ".form-tab", "Invite Link")).click();
  await (await shown(b, ".add-friend-generate .form-btn-primary")).click();
  await resolveIntent(b, { passphrase: s.passphrase });
  await shown(b, ".add-friend-invite-result", NET_WAIT_MS);
}

async function statusLabel(b: Browser): Promise<string> {
  return (await b.$(".status-picker .buddy-name")).getText();
}

const BASE_ROWS: Row[] = [
  {
    id: "L3",
    title: "Create account mode",
    async run({ b }) {
      await switchTo(b, LOGIN, NET_WAIT_MS);
      if (await present(b, ".account-create-btn")) await (await b.$(".account-create-btn")).click();
      await shown(b, 'input[placeholder="Display Name (optional)"]');
      return "create form shown (first run opens it without a picker)";
    },
  },
  {
    id: "L5",
    title: "Back (first run)",
    async run({ b }) {
      if (!(await present(b, "button.form-btn-secondary"))) {
        throw new Skip("no Back on first run (LoginWindow.tsx:211 shows it only with identities); walked again after logout");
      }
      await (await byText(b, "button.form-btn-secondary", "Back")).click();
      await shown(b, ".account-picker");
    },
  },
  {
    id: "L6",
    title: "Create identity",
    async run({ b, s }) {
      await createIdentity(b, s);
    },
  },
  {
    id: "B12",
    title: "Edit own name",
    async run({ b, s }) {
      await inBuddy(b);
      const next = `${s.displayName}-x`;
      await (await shown(b, ".identity-bar-name")).click();
      const edit = await shown(b, ".identity-bar-edit");
      await typeInto(edit, next);
      const probe = async (): Promise<string> => `${await identityBarState(b)}; ${await pageLog(b)}`;
      await recordPage(b, ["set_nickname", "get_identity"], ["selfProfileChanged", "intentPending", "selfChanged"]);
      const typed = await probe();
      await b.keys(Key.Enter);
      const entered = await probe();
      const outcome = await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await (await b.$(".identity-bar-name")).getText()).includes(next), WAIT_MS, "name did not change").catch(async (e: Error) => {
        throw new Error(`${e.message} (typed: ${typed}; after Enter: ${entered}; intent ${outcome}; now: ${await probe()})`);
      });
      s.displayName = next;
    },
  },
  {
    id: "B13",
    title: "Copy own public key",
    async run({ b }) {
      await (await shown(b, ".identity-bar-key")).click();
      throw new Skip("clicked; the UI shows no copied feedback (UserIdentityBar.tsx:101, buddy.actions.ts:127)");
    },
  },
  {
    id: "B14",
    title: "Edit status message",
    async run({ b, s }) {
      const msg = "walking the app";
      await (await shown(b, ".identity-bar-status-msg")).click();
      await typeInto(await shown(b, ".identity-bar-status-msg-edit"), msg);
      await b.keys(Key.Enter);
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await waitUntil(async () => (await (await b.$(".identity-bar-status-msg")).getText()).includes(msg), WAIT_MS, "status message did not change");
    },
  },
  {
    id: "B15",
    title: "Open notification center",
    async run({ b }) {
      const bell = await shown(b, ".notification-bell");
      if (!(await present(b, ".notification-panel"))) await bell.click();
      await shown(b, ".notification-panel");
      if ((await bell.getAttribute("aria-expanded")) !== "true") throw new Error("bell aria-expanded is not true");
    },
  },
  {
    id: "B16",
    title: "Mark all notifications read",
    async run({ b }) {
      if (!(await present(b, ".notification-panel"))) await (await b.$(".notification-bell")).click();
      await shown(b, ".notification-panel");
      if (!(await present(b, ".notification-mark-all"))) {
        await (await b.$(".notification-bell")).click();
        throw new Skip("no unread notifications, so no Mark all read button (NotificationCenter.tsx:62)");
      }
      await (await b.$(".notification-mark-all")).click();
      await gone(b, ".notification-mark-all");
      await (await b.$(".notification-bell")).click();
    },
  },
  {
    id: "B20",
    title: "Friends / Communities tabs, Alt+1 / Alt+2",
    async run({ b }) {
      const active = async (prefix: string): Promise<boolean> =>
        ((await (await b.$(`.buddy-tab[title^="${prefix}"]`)).getAttribute("class")) ?? "").includes("buddy-tab-active");
      await (await shown(b, '.buddy-tab[title^="Communities"]')).click();
      await waitUntil(() => active("Communities"), WAIT_MS, "click did not activate Communities");
      await (await b.$(".buddy-search-input")).click();
      await b.keys([Key.Alt, "1"]);
      await waitUntil(() => active("Friends"), WAIT_MS, "Alt+1 did not activate Friends");
      await b.keys([Key.Alt, "2"]);
      await waitUntil(() => active("Communities"), WAIT_MS, "Alt+2 did not activate Communities");
      await (await b.$('.buddy-tab[title^="Friends"]')).click();
      await waitUntil(() => active("Friends"), WAIT_MS, "click did not activate Friends");
    },
  },
  {
    id: "B21",
    title: "Search filter, Ctrl/Cmd+F",
    async run({ b }) {
      await (await shown(b, ".identity-bar")).click();
      await b.keys([MOD_KEY, "f"]);
      await waitUntil(
        async () => b.execute(() => document.activeElement?.classList.contains("buddy-search-input") ?? false),
        WAIT_MS,
        "Ctrl/Cmd+F did not focus the search input",
      );
      const input = await b.$(".buddy-search-input");
      await input.addValue("zzz-no-match");
      if ((await input.getValue()) !== "zzz-no-match") throw new Error("search input did not take the text");
      await sleep(300);
      await input.clearValue();
      await input.addValue(" ");
      await input.clearValue();
    },
  },
  {
    id: "B22",
    title: "Dismiss pairing-link notice",
    async run({ b }) {
      const notice = '.sec-review[role="status"]';
      if (!(await present(b, notice))) throw new Skip("notice not shown: only after a refused pairing link (PairingLinkNotice.tsx)");
      await (await b.$(`${notice} .sec-btn-secondary`)).click();
      await gone(b, notice);
    },
  },
  {
    id: "B45",
    title: "Collapse / expand a group",
    async run({ b }) {
      if (!(await present(b, ".buddy-group-header"))) throw new Skip("no group: a fresh identity has no friends or outgoing requests");
      const header = await b.$(".buddy-group-header");
      const before = await header.getText();
      await header.click();
      await waitUntil(async () => (await header.getText()) !== before, WAIT_MS, "group arrow did not toggle");
      await header.click();
    },
  },
  {
    id: "B48",
    title: "Add Friend modal and its tabs",
    async run({ b }) {
      await openAddFriend(b);
      await (await byText(b, ".form-tab", "Public Key")).click();
      await shown(b, 'input[aria-label="Friend code"]');
      await (await byText(b, ".form-tab", "Invite Link")).click();
      await shown(b, ".add-friend-generate");
    },
  },
  {
    id: "B50",
    title: "Copy invite link",
    async run(ctx) {
      const { b } = ctx;
      await generateInvite(ctx);
      const copy = await shown(b, ".add-friend-copy-btn");
      await copy.click();
      await waitUntil(async () => (await copy.getText()).includes("Copied!"), 3000, "no Copied! feedback");
      return "generated an invite first (B49) to have one to copy";
    },
  },
  {
    id: "B51",
    title: "Cancel current invite",
    async run({ b, s }) {
      if (!(await present(b, ".add-friend-cancel-btn"))) throw new Error("no current invite (B50 failed)");
      await (await b.$(".add-friend-cancel-btn")).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await gone(b, ".add-friend-invite-result", NET_WAIT_MS);
    },
  },
  {
    id: "B52",
    title: "Cancel invite from the list",
    async run(ctx) {
      const { b, s } = ctx;
      // "Active Invites" lists every live invite but the current one
      // (InviteLinkTab.tsx:86-89, :121), so it needs two.
      await generateInvite(ctx);
      await generateInvite(ctx);
      await shown(b, ".invite-cancel-btn", NET_WAIT_MS);
      const before = (await b.$$(".add-friend-invite-item")).length;
      await (await b.$(".invite-cancel-btn")).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await waitUntil(async () => (await b.$$(".add-friend-invite-item")).length < before, NET_WAIT_MS, "list did not shrink");
      return `generated two invites; list ${before} -> fewer`;
    },
  },
  {
    id: "B54",
    title: "Show own friend code",
    async run({ b }) {
      await openAddFriend(b);
      await (await byText(b, ".form-tab", "Public Key")).click();
      const url = await shown(b, "output.sec-friend-code-url", NET_WAIT_MS);
      if (!(await url.getText()).trim()) throw new Error("friend code is empty");
    },
  },
  {
    id: "B55",
    title: "Copy friend code",
    async run({ b }) {
      const copy = await byText(b, ".sec-friend-code-actions button", "Copy code");
      await copy.click();
      await waitUntil(async () => (await copy.getText()).includes("Copied!"), 3000, "no Copied! feedback");
    },
  },
  {
    id: "B56",
    title: "Rotate friend code",
    async run({ b, s }) {
      const url = await shown(b, "output.sec-friend-code-url");
      const before = await url.getText();
      await (await byText(b, ".sec-friend-code-actions button", "New code")).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await waitUntil(async () => (await (await b.$("output.sec-friend-code-url")).getText()) !== before, NET_WAIT_MS, "friend code did not change");
      await closeModal(b);
    },
  },
  {
    id: "B60",
    title: "Create community",
    async run({ b, s }) {
      await closeModal(b);
      // A create before the node is on the network is refused (walk 15: 409
      // "cannot perform write operation in state 'detached'", then, attached,
      // "set_dht_value: … not stored (Offline)"); the user waits for the
      // indicator's "Connected" (NetworkIndicator.tsx:66, public internet
      // ready and a route).
      await waitUntil(
        async () => (await (await b.$(".network-indicator span")).getText()).trim() === "Connected",
        NET_WAIT_MS,
        "the network indicator never said Connected",
      );
      // The Create Community button shows only on the Communities tab
      // (BottomActionBar.tsx:45-46).
      await (await shown(b, '.buddy-tab[title^="Communities"]')).click();
      await (await shown(b, '[aria-label="Create Community"]')).click();
      await typeInto(await shown(b, 'input[placeholder="Community name..."]'), s.communityName);
      // B60 failed twice with the create rejected 139 ms after its review (walk 14, 15): record who settled it.
      await recordPage(b, ["create_community", "intent_commit", "intent_cancel"], ["community", "intentPending"]);
      await (await b.$('.modal-container button[type="submit"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await gone(b, 'input[placeholder="Community name..."]', NET_WAIT_MS);
      await (await b.$('.buddy-tab[title^="Communities"]')).click();
      await byText(b, ".community-item", s.communityName, NET_WAIT_MS).catch(async (e: Error) => {
        throw new Error(`${e.message} (${await pageLog(b)})`);
      });
    },
  },
  {
    id: "B62",
    title: "Double-click community",
    async run({ b, s, shot }) {
      await inBuddy(b);
      await (await b.$('.buddy-tab[title^="Communities"]')).click();
      const before = await b.getWindowHandles();
      await doubleClick(b, await byText(b, ".community-item", s.communityName, NET_WAIT_MS));
      const handle = await waitForHandle(b, (h) => h.startsWith("community-") && !before.includes(h), WAIT_MS);
      await b.switchToWindow(handle);
      await shown(b, "body");
      await sleep(1000);
      await shot();
      await b.closeWindow();
      await inBuddy(b);
      await (await b.$('.buddy-tab[title^="Friends"]')).click();
      return `opened ${handle}; dblclick dispatched in page (the server synthesises none)`;
    },
  },
  {
    id: "B63",
    title: "Status picker: Away, Busy, Appear Offline, Online",
    async run({ b, s }) {
      const seen: string[] = [];
      // The Host's answer to each pick and our own presence events, named
      // in the note when a label never shows (B63 failed on Online in walks
      // 8, 21-23 after passing the other three).
      await recordPage(b, ["set_status"], ["selfChanged", "SelfChanged"]);
      // Each step has its own deadline, so a stuck call fails the row with
      // its name instead of running on into B9 after the row timeout.
      for (const label of ["Away", "Busy", "Appear Offline", "Online"]) {
        const step = <T,>(name: string, work: Promise<T>): Promise<T> => within(work, 30_000, `${label}: ${name} did not return in 30 s`);
        await step("press trigger", press(b, await step("find trigger", shown(b, ".status-picker"))));
        await step("dropdown", shown(b, ".status-picker-dropdown"));
        await step("press item", press(b, await step("find item", byText(b, ".status-picker-dropdown .context-menu-item", label))));
        await step("intent", resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 }));
        await step("dropdown closes", gone(b, ".status-picker-dropdown"));
        await step("label", waitUntil(async () => (await statusLabel(b)).includes(label), WAIT_MS, `status did not show ${label}`).catch(async (e: Error) => {
          throw new Error(`${e.message}; label shows "${await statusLabel(b)}" (${await pageLog(b)})`);
        }));
        seen.push(await step("read label", statusLabel(b)));
      }
      return `shown: ${seen.join(", ")}`;
    },
  },
  {
    id: "B9",
    title: "Menu > Calls > Start group call",
    async run({ b, shot }) {
      await menu(b, "Calls", "Start Group Call");
      await byText(b, ".modal-title", "Start Group Call");
      await shot();
      await (await byText(b, ".modal-footer button", "Cancel")).click();
      await gone(b, ".modal-footer");
    },
  },
  {
    id: "B10",
    title: "Help > Documentation (review cancelled)",
    async run({ b, s, shot }) {
      await menu(b, "Help", "Documentation");
      await shown(b, ".intent-dialog", WAIT_MS);
      await shot();
      if ((await resolveIntent(b, { passphrase: s.passphrase, cancel: true })) !== "cancelled") throw new Error("review not cancelled");
    },
  },
  {
    id: "B11",
    title: "Help > Report bug (review cancelled)",
    async run({ b, s, shot }) {
      await menu(b, "Help", "Report Bug");
      await shown(b, ".intent-dialog", WAIT_MS);
      await shot();
      if ((await resolveIntent(b, { passphrase: s.passphrase, cancel: true })) !== "cancelled") throw new Error("review not cancelled");
    },
  },
  settingsRow("B2", "Profile"),
  settingsRow("B3", "Application"),
  settingsRow("B4", "Notifications"),
  settingsRow("B5", "Audio"),
  settingsRow("B6", "Privacy"),
  settingsRow("B7", "About"),
  // Rows in rows-settings.ts, rows-community.ts and rows-peer.ts run here,
  // while the identity is logged in (the getters keep the imports lazy).
  {
    id: "B1",
    title: "Titlebar minimize / close (hides)",
    async run({ b, shot }) {
      await inBuddy(b);
      const rect = await b.getWindowRect();
      await (await b.$('[aria-label="Minimize window"]')).click();
      await sleep(1000);
      const minimized = await b.execute(() => document.visibilityState);
      await b.setWindowRect(rect.x, rect.y, rect.width, rect.height);
      await sleep(500);
      await (await b.$('[aria-label="Hide window"]')).click();
      await sleep(1000);
      const hidden = await b.execute(() => document.visibilityState);
      const stillListed = (await b.getWindowHandles()).includes(BUDDY);
      // Recovery: the same command the login window calls to show the list.
      await b.execute(() => {
        const internals = (window as unknown as { __TAURI_INTERNALS__?: { invoke(cmd: string): Promise<unknown> } }).__TAURI_INTERNALS__;
        void internals?.invoke("show_buddy_list");
      });
      await sleep(1000);
      await shot();
      const note = `after minimize visibilityState=${minimized}; after hide visibilityState=${hidden}, handle kept=${stillListed}; restored via setWindowRect and show_buddy_list`;
      if (minimized !== "hidden" || hidden !== "hidden" || !stillListed) throw new Error(note);
      return note;
    },
  },
  {
    id: "B64",
    title: "Logout",
    async run({ b, s }) {
      await inBuddy(b);
      await (await b.$('[aria-label="Logout"]')).click();
      await resolveIntent(b, { passphrase: s.passphrase });
      await switchTo(b, LOGIN, NET_WAIT_MS);
      const name = await (await shown(b, ".lock-name", NET_WAIT_MS)).getText();
      if (name.trim() !== s.displayName) throw new Error(`unlock screen shows ${name}`);
      await toPicker(b);
      return "logged out to the unlock screen, then Switch Account to the picker";
    },
  },
  {
    id: "L3b",
    title: "Create account mode (from the picker)",
    async run({ b }) {
      await switchTo(b, LOGIN);
      await (await shown(b, ".account-create-btn")).click();
      await shown(b, 'input[placeholder="Display Name (optional)"]');
    },
  },
  {
    id: "L5b",
    title: "Back (to the picker)",
    async run({ b }) {
      await (await byText(b, "button.form-btn-secondary", "Back")).click();
      await shown(b, ".account-picker");
      return "Back matched by text: no aria-label (LoginWindow.tsx:221)";
    },
  },
  {
    id: "L1",
    title: "Select account bubble",
    async run({ b, s }) {
      await (await byText(b, ".account-bubble", s.displayName)).click();
      const name = await (await shown(b, ".lock-name")).getText();
      if (!name.includes(s.displayName)) throw new Error(`lock screen shows "${name}"`);
    },
  },
  {
    id: "L4",
    title: "Login with passphrase",
    async run({ b, s }) {
      await typeInto(await shown(b, 'form.login-container input[type="password"]'), s.passphrase);
      const submit = await b.$('form.login-container button[type="submit"]');
      await submit.waitForEnabled({ timeout: NET_WAIT_MS, timeoutMsg: "Unlock stayed disabled (Connecting…)" });
      await submit.click();
      await resolveIntent(b, { passphrase: s.passphrase, appearMs: 1500 });
      await inBuddy(b);
    },
  },
  {
    id: "L2",
    title: "Delete account",
    async run({ b, s }) {
      const handles = await b.getWindowHandles();
      if (handles.includes(BUDDY) && !(await (async () => {
        await b.switchToWindow(LOGIN).catch(() => undefined);
        return present(b, ".account-bubble");
      })())) {
        await b.switchToWindow(BUDDY);
        await (await b.$('[aria-label="Logout"]')).click();
        await resolveIntent(b, { passphrase: s.passphrase });
      }
      await toPicker(b);
      // The delete button is opacity 0 until the bubble is hovered or holds
      // focus (theme/01-form-primitives.css, `.account-bubble-delete`), so
      // WebDriver never sees it displayed: wait for it to exist and press it
      // in the page.
      const del = await b.$(`[aria-label="Delete ${s.displayName}"]`);
      await del.waitForExist({ timeout: NET_WAIT_MS });
      await press(b, del);
      const outcome = await resolveIntent(b, { passphrase: s.passphrase, appearMs: WAIT_MS });
      await waitUntil(async () => !(await present(b, `[aria-label="Delete ${s.displayName}"]`)), NET_WAIT_MS, "account bubble still shown");
      return `review ${outcome}`;
    },
  },
  {
    id: "B8",
    title: "Menu > Quit (inventory W3)",
    // Last: quitting ends the walk. L2 deleted the walk's identity, so make
    // a fresh one to reach the buddy list's menu bar.
    async run({ b, app, s, shot }) {
      await createIdentity(b, s);
      await shot();
      const started = Date.now();
      // Quit takes the WebDriver server down with the app, so the helper's
      // follow-up call after the click can be refused: that is the exit this
      // row asserts, not a failure. Any error while the process lives is.
      const clicked = await menu(b, "Rekindle", "Quit").then(
        () => null,
        (e: unknown) => e,
      );
      // The process exits (`quit_app`, the tray's Quit), after the graceful
      // shutdown; closing the window alone would leave it in the tray.
      const deadline = started + NET_WAIT_MS;
      while (app.alive() && Date.now() < deadline) await sleep(250);
      if (app.alive()) {
        if (clicked !== null) throw clicked;
        const handles = await b.getWindowHandles().catch(() => [] as string[]);
        throw new Error(`did not quit: process alive; windows=[${handles.join(", ")}]`);
      }
      return `process exited ${Date.now() - started} ms after Quit`;
    },
  },
];

const SPLICE_AFTER = "B7";

/** The walk's rows in order: rows.ts's own, with the per-window files after B7. */
export async function allRows(): Promise<Row[]> {
  const { SETTINGS_ROWS } = await import("./rows-settings.ts");
  const { COMMUNITY_ROWS } = await import("./rows-community.ts");
  const { PEER_ROWS } = await import("./rows-peer.ts");
  const { PEER_COMMUNITY_ROWS } = await import("./rows-peer-community.ts");
  const { VOICE_ROWS } = await import("./rows-voice.ts");
  const at = BASE_ROWS.findIndex((r) => r.id === SPLICE_AFTER) + 1;
  // The voice rows need the peer in the walk's community (B61) and end
  // before the member-menu rows that kick it (M88 → M86).
  const voiceAt = PEER_COMMUNITY_ROWS.findIndex((r) => r.id === "M88");
  const peerCommunity = [...PEER_COMMUNITY_ROWS.slice(0, voiceAt), ...VOICE_ROWS, ...PEER_COMMUNITY_ROWS.slice(voiceAt)];
  return [...BASE_ROWS.slice(0, at), ...SETTINGS_ROWS, ...COMMUNITY_ROWS, ...PEER_ROWS, ...peerCommunity, ...BASE_ROWS.slice(at)];
}
