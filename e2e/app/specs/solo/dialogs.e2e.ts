// Plan E7.14 S5 (moved from e2e/dialog-focus.spec.ts and
// e2e/keyboard-shortcuts.spec.ts; plan E7.3, wiring guide §13c and §13f):
// every modal dialog takes focus when it opens, Esc closes it, and focus
// goes back to the control that opened it; the keyboard-shortcuts dialog
// lists the app's chords, opens from the buddy list's action bar and with
// Mod+/ in any window. Each opener is focused, then activated, as a keyboard
// user does (WebKit on macOS does not focus a button on a mouse click).
//
// Tab and Shift+Tab staying inside each dialog need the engine's own focus
// move, which only native key input gives (native-key.ts): on a platform
// without it (macOS) that test is skipped with the driver's reason, and
// the focus trap is an owed manual check there.

import { readFileSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, press, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { createCommunity, createIdentity, createTextChannel, newUser, openCommunity, postMessage, selectChannel, type User } from "../../journeys.ts";
import { nativeKey } from "../../native-key.ts";

const SHORTCUTS_RS = path.join(import.meta.dirname, "..", "..", "..", "..", "src-tauri", "src", "shortcuts.rs");

/** The OS-wide chords src-tauri/src/shortcuts.rs registers on this platform, as the dialog spells them. */
function globalChords(mac: boolean): string[] {
  const lines = readFileSync(SHORTCUTS_RS, "utf8").split("\n");
  const cfg = mac ? '#[cfg(target_os = "macos")]' : '#[cfg(not(target_os = "macos"))]';
  const at = lines.findIndex((l) => l.trim() === cfg);
  if (at < 0) throw new Error(`shortcuts.rs has no ${cfg} line`);
  const chords = [...lines[at + 1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (chords.length === 0) throw new Error(`shortcuts.rs lists no chords after ${cfg}`);
  const label: Record<string, string> = { super: "Cmd", ctrl: "Ctrl", shift: "Shift", alt: "Alt" };
  return chords.map((c) => c.split("+").map((k) => label[k] ?? k.toUpperCase()).join("+"));
}

/** Whether focus is inside the open dialog matching `dialog`. */
async function focusInside(dialog: string): Promise<boolean> {
  return (await browser.execute((sel: string) => {
    const d = document.querySelector(sel);
    return !!d && d.contains(document.activeElement);
  }, dialog)) as boolean;
}

/** Whether `opener` holds focus. */
async function focused(opener: string): Promise<boolean> {
  return (await browser.execute((sel: string) => document.activeElement === document.querySelector(sel), opener)) as boolean;
}

/**
 * Focus `opener` and activate it; the dialog `dialog` opens with focus
 * inside; Esc closes it and focus returns to `opener`.
 */
async function escReturnsFocus(opener: string, dialog: string): Promise<void> {
  // Exists, not shown: a message's action bar is collapsed until hover or
  // focus within it (10-thread-list-panel.css), and focusing reveals it.
  const el = await browser.$(opener);
  await el.waitForExist({ timeout: NET_WAIT_MS, timeoutMsg: `${opener} never existed` });
  await browser.execute((node: HTMLElement) => node.focus(), el as unknown as HTMLElement);
  await waitUntil(() => focused(opener), NET_WAIT_MS, `${opener} did not take focus`);
  await press(browser, el);
  await shown(browser, dialog, NET_WAIT_MS);
  await waitUntil(() => focusInside(dialog), NET_WAIT_MS, `${dialog} never took focus`);
  await browser.keys(["Escape"]);
  await gone(browser, dialog, NET_WAIT_MS);
  await waitUntil(() => focused(opener), NET_WAIT_MS, `focus did not return to ${opener}`);
}

describe("dialogs take focus, close on Esc and give focus back", function () {
  this.timeout(600_000);
  const me: User = newUser("dialogs");
  let communityHandle = "";

  before(async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Dialogs");
    await openCommunity(browser, "Dialogs");
    communityHandle = await browser.getWindowHandle();
    await createTextChannel(browser, me, "general");
    await selectChannel(browser, "general");
    await postMessage(browser, me, "message 1 with some ordinary words");
  });

  it("the community search panel: focus in, Esc closes it, focus back on Search", async () => {
    await browser.switchToWindow(communityHandle);
    await escReturnsFocus('[aria-label="Search community"]', '[role="dialog"]');
  });

  it("the forward dialog: focus in, Esc closes it, focus back on Forward", async () => {
    await browser.switchToWindow(communityHandle);
    await escReturnsFocus('.community-main .chat-message [aria-label="Forward message"]', '[role="dialog"]');
  });

  it("Add Friend (a shared Modal): focus in, Esc closes it, focus back on Add Friend", async () => {
    await switchTo(browser, BUDDY);
    // Add Friend shows on the Friends tab only (BottomActionBar.tsx).
    await (await shown(browser, '.buddy-tab[title^="Friends"]')).click();
    await escReturnsFocus('[aria-label="Add Friend"]', '[role="dialog"]');
  });

  it("the action bar opens the shortcuts, which list the global chords; Esc gives focus back", async () => {
    await switchTo(browser, BUDDY);
    const opener = '[aria-label="Keyboard shortcuts"]';
    const el = await shown(browser, opener, NET_WAIT_MS);
    await browser.execute((node: HTMLElement) => node.focus(), el as unknown as HTMLElement);
    await press(browser, el);
    const dialog = '[role="dialog"]';
    await shown(browser, dialog, NET_WAIT_MS);
    const listed = (await browser.execute(
      () => Array.from(document.querySelectorAll('[role="dialog"] .shortcuts-help-chord')).map((k) => (k.textContent ?? "").trim()),
    )) as string[];
    const missing = globalChords(process.platform === "darwin").filter((c) => !listed.includes(c));
    expect(missing).toEqual([]);
    const text = (await browser.execute(() => document.querySelector('[role="dialog"]')?.textContent ?? "")) as string;
    expect(text).toContain("Show or hide the buddy list");
    expect(text).toContain("Shift+F10");
    await browser.keys(["Escape"]);
    await gone(browser, dialog, NET_WAIT_MS);
    await waitUntil(() => focused(opener), NET_WAIT_MS, "focus did not return to Keyboard shortcuts");
  });

  it("Mod+/ opens the shortcuts in a community window; Esc gives focus back to the composer", async () => {
    await browser.switchToWindow(communityHandle);
    const composer = ".community-main .message-input-field";
    const el = await shown(browser, composer, NET_WAIT_MS);
    await browser.execute((node: HTMLElement) => node.focus(), el as unknown as HTMLElement);
    const mod = process.platform === "darwin" ? "Meta" : "Control";
    await browser.keys([mod, "/"]);
    await browser.keys([mod]);
    await shown(browser, '[role="dialog"]', NET_WAIT_MS);
    await browser.keys(["Escape"]);
    await gone(browser, '[role="dialog"]', NET_WAIT_MS);
    await waitUntil(() => focused(composer), NET_WAIT_MS, "focus did not return to the composer");
  });

  it("Tab and Shift+Tab stay inside a dialog (native keys)", async function () {
    await switchTo(browser, BUDDY);
    await (await shown(browser, '.buddy-tab[title^="Friends"]')).click();
    const el = await shown(browser, '[aria-label="Add Friend"]', NET_WAIT_MS);
    await browser.execute((node: HTMLElement) => node.focus(), el as unknown as HTMLElement);
    await press(browser, el);
    const dialog = '[role="dialog"]';
    await shown(browser, dialog, NET_WAIT_MS);
    await waitUntil(() => focusInside(dialog), NET_WAIT_MS, "the dialog never took focus");
    const first = await nativeKey(browser, "Tab");
    if (first !== "sent") {
      await browser.keys(["Escape"]);
      console.log(`[skip] native keys are not available here: ${first.unsupported}`);
      this.skip();
    }
    const escaped: string[] = [];
    for (const shift of [false, true]) {
      for (let i = 0; i < 14; i++) {
        if (i > 0 || shift) await nativeKey(browser, "Tab", shift);
        if (!(await focusInside(dialog))) escaped.push(`${shift ? "Shift+Tab" : "Tab"} #${i + 1}`);
      }
    }
    await browser.keys(["Escape"]);
    expect(escaped).toEqual([]);
  });
});
