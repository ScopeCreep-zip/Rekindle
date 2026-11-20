// Plan E7.13: the rows that end the walk, from
// .claude/plans/standards-remediation/evidence/app-journeys-2026-11-17.md,
// split from rows.ts by size. They run after the per-window files, on the
// identity rows.ts created: hide and restore the buddy list, log out, walk
// the picker and lock screen back in, delete the identity, and Quit last.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, byText, present, press, resolveIntent, shown, sleep, switchTo, typeInto, waitUntil } from "./helpers.ts";
import { BUDDY, LOGIN, inBuddy, menu } from "./nav.ts";
import { type Row, createIdentity } from "./rows.ts";

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

export const SESSION_ROWS: Row[] = [
  {
    id: "B1",
    title: "Titlebar minimize / close (hides)",
    async run({ b, shot }) {
      await inBuddy(b);
      const rect = await b.getWindowRect();
      await (await b.$('[aria-label="Minimize window"]')).click();
      await sleep(1000);
      const minimized = await b.execute(() => document.visibilityState);
      const keptMinimized = (await b.getWindowHandles()).includes(BUDDY);
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
      // Wayland reports no minimized state back to a client: xdg-shell's
      // xdg_toplevel.set_minimized says "There is no way to know if the
      // surface is currently minimized", so GDK never sets ICONIFIED, tao's
      // is_minimized stays false (tao-0.37.1 platform_impl/linux/window.rs:
      // 369-372) and WebKitGTK keeps the page "visible" (COSMIC walk of
      // 9fd296f4). There the row proves what the platform shows: the
      // minimize click keeps the window, which restores. X11 and macOS
      // report the state, so the visibility check stays there.
      const waylandMinimize = process.platform === "linux" && Boolean(process.env.WAYLAND_DISPLAY);
      const minimizeShown = waylandMinimize ? keptMinimized : minimized === "hidden" && keptMinimized;
      const note =
        `after minimize visibilityState=${minimized}${waylandMinimize ? " (Wayland: no minimized state reported)" : ""}, handle kept=${keptMinimized}; ` +
        `after hide visibilityState=${hidden}, handle kept=${stillListed}; restored via setWindowRect and show_buddy_list`;
      if (!minimizeShown || hidden !== "hidden" || !stillListed) throw new Error(note);
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
