// Plan E7.13: navigation shared by the walk's row files (rows.ts,
// rows-settings.ts, rows-community.ts): the window labels, the buddy list's
// menu bar, the Settings window and modals.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, byText, gone, present, press, resolveIntent, shown, switchTo, waitUntil } from "./helpers.ts";

export const LOGIN = "login";
export const BUDDY = "buddy-list";
export const SETTINGS = "settings";

export async function inBuddy(b: Browser): Promise<void> {
  await switchTo(b, BUDDY, NET_WAIT_MS);
  await shown(b, ".identity-bar", NET_WAIT_MS);
}

/** Open a buddy-list menu (Kobalte Menubar) and choose one of its items. */
export async function menu(b: Browser, trigger: string, item: string): Promise<void> {
  await press(b, await byText(b, ".menu-bar-item", trigger));
  await shown(b, ".menu-dropdown");
  await press(b, await byText(b, ".menu-dropdown-item", item));
}

export async function closeModal(b: Browser): Promise<void> {
  if (await present(b, ".modal-close-btn")) {
    await (await b.$(".modal-close-btn")).click();
    await gone(b, ".modal-container");
  }
}

/**
 * Open Settings from Menu > Rekindle > Profile and select `tab` in its tab
 * rail (SettingsWindow.tsx:66-77): the menu names six of the ten tabs.
 */
export async function openSettings(b: Browser, tab: string): Promise<void> {
  await inBuddy(b);
  await menu(b, "Rekindle", "Profile");
  await switchTo(b, SETTINGS);
  await (await byText(b, ".form-tab", tab)).click();
  await waitUntil(
    async () => (await (await b.$(".form-tab-active")).getText()).trim() === tab,
    WAIT_MS,
    `Settings tab ${tab} did not become active`,
  );
}

export async function closeSettings(b: Browser): Promise<void> {
  await b.closeWindow();
  await inBuddy(b);
}

/** The checkbox of the `label.settings-option` whose text contains `text`. */
export async function optionBox(b: Browser, text: string): Promise<Awaited<ReturnType<Browser["$"]>>> {
  const label = await byText(b, "label.settings-option, label.settings-toggle-row", text);
  return label.$("input[type=checkbox]");
}

/**
 * Flip a Settings checkbox, confirm the switch shows it, then close and
 * reopen Settings and confirm the stored value came back; flip it back the
 * same way. Returns the two observed states.
 */
export async function flipPersisted(b: Browser, tab: string, text: string, passphrase: string): Promise<string> {
  const flip = async (): Promise<boolean> => {
    const box = await optionBox(b, text);
    const before = await box.isSelected();
    await box.click();
    await resolveIntent(b, { passphrase, appearMs: 1500 });
    await waitUntil(async () => (await (await optionBox(b, text)).isSelected()) !== before, WAIT_MS, `"${text}" did not flip`);
    await closeSettings(b);
    await openSettings(b, tab);
    const stored = await (await optionBox(b, text)).isSelected();
    if (stored === before) throw new Error(`"${text}" read back ${stored} after reopening Settings`);
    return stored;
  };
  const first = await flip();
  const second = await flip();
  return `"${text}": ${!first} -> ${first} (kept after reopen) -> ${second} (kept after reopen)`;
}

/** Set a `<select>` / `<input>` value the way a user's choice does: the value, then `input` and `change`. */
export async function setValue(b: Browser, selector: string, value: string): Promise<void> {
  const el = await shown(b, selector);
  await b.execute(
    (node: HTMLInputElement, v: string) => {
      node.value = v;
      node.dispatchEvent(new Event("input", { bubbles: true }));
      node.dispatchEvent(new Event("change", { bubbles: true }));
    },
    el as unknown as HTMLInputElement,
    value,
  );
}

/**
 * The next `<input type=file>.click()` in this window receives `file` as if
 * the user picked it in the OS dialog, which WebDriver cannot drive: the
 * page's own change handler then runs on it.
 */
export async function pickNextFile(b: Browser, name: string, mime: string, base64: string): Promise<void> {
  await b.execute(
    (n: string, m: string, data: string) => {
      const proto = HTMLInputElement.prototype;
      const original = proto.click;
      proto.click = function (this: HTMLInputElement) {
        if (this.type !== "file") return original.call(this);
        proto.click = original;
        const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
        const dt = new DataTransfer();
        dt.items.add(new File([bytes], n, { type: m }));
        this.files = dt.files;
        this.dispatchEvent(new Event("change", { bubbles: true }));
      };
    },
    name,
    mime,
    base64,
  );
}
