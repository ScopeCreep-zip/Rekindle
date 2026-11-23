// Plan E7.13: navigation shared by the walk's row files (rows.ts,
// rows-settings.ts, rows-community.ts): the window labels, the buddy list's
// menu bar, the Settings window and modals.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, WAIT_MS, byText, gone, present, press, resolveIntent, shown, switchTo, typeInto, waitUntil } from "./helpers.ts";

export const LOGIN = "login";
export const BUDDY = "buddy-list";
export const SETTINGS = "settings";

/**
 * The buddy list, settled on its layout: its skin's sheet linked, then
 * either the skin's tree (plan SK2: the pack's nodes, found by their
 * `data-skin-role` / `data-skin-action`, the same in every pack) or the
 * app's own bar. Deciding before the sheet links raced the tree replacing
 * the bar mid-step (the SK2 capture, 2026-11-21).
 */
export async function inBuddy(b: Browser): Promise<void> {
  await switchTo(b, BUDDY, NET_WAIT_MS);
  await waitUntil(
    async () =>
      (await b.execute(() => {
        const l = document.getElementById("rekindle-skin") as HTMLLinkElement | null;
        if (!l?.sheet) return false;
        // A tree with its nodes laid (not every pack lays a name field: Total
        // Chaos and xSLIMFIRE have none), or the app's own bar.
        return !!document.querySelector(".skin-tree [data-skin-node]") || !!document.querySelector(".identity-bar");
      })) as boolean,
    NET_WAIT_MS,
    "the buddy list showed neither its skin's tree nor its own bar",
  );
}

/** Whether the buddy list is drawn as its skin's tree (call after `inBuddy`). */
export async function underTree(b: Browser): Promise<boolean> {
  return present(b, ".skin-tree");
}

/** The engine menu a buddy-list menu title opens (design §2.2 `MenuRegistry`). */
const ENGINE_MENU: Record<string, string> = { Rekindle: "file", Tools: "tool", Help: "help" };

/**
 * Open a buddy-list menu and choose one of its items: the pack's own menu
 * title (`data-skin-action="menu:<engine menu>"`, its items the Host's
 * registry) under a skin's tree, else the app's menu bar (Kobalte Menubar).
 */
export async function menu(b: Browser, trigger: string, item: string): Promise<void> {
  if (await underTree(b)) {
    const engine = ENGINE_MENU[trigger];
    if (!engine) throw new Error(`no engine menu for the "${trigger}" menu`);
    await press(b, await shown(b, `[data-skin-action="menu:${engine}"]`));
  } else {
    await press(b, await byText(b, ".menu-bar-item", trigger));
  }
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

/** One Settings tab as painted: its label, `aria-selected`, active class and underline colour. */
interface TabPaint {
  text: string;
  selected: string | null;
  active: boolean;
  underline: string;
}

const TRANSPARENT = "rgba(0, 0, 0, 0)";

/**
 * Finding V4: after a click the Settings tab rail must mark exactly the
 * clicked tab, in the accessibility tree (`role="tab"`, `aria-selected`,
 * WCAG 4.1.2), in the class and in what is painted (its underline's
 * computed colour); every other tab is unselected with no underline.
 */
export async function expectOnlySelectedTab(b: Browser, tab: string): Promise<void> {
  let seen: TabPaint[] = [];
  const ok = async (): Promise<boolean> => {
    seen = await b.execute(() =>
      Array.from(document.querySelectorAll<HTMLElement>(".form-tabs .form-tab")).map((el) => ({
        text: (el.textContent ?? "").trim(),
        selected: el.getAttribute("aria-selected"),
        active: el.classList.contains("form-tab-active"),
        underline: getComputedStyle(el).borderBottomColor,
      })),
    );
    return (
      seen.length > 0 &&
      seen.every((t) =>
        t.text === tab
          ? t.selected === "true" && t.active && t.underline !== TRANSPARENT
          : t.selected === "false" && !t.active && t.underline === TRANSPARENT,
      )
    );
  };
  const deadline = Date.now() + WAIT_MS;
  while (!(await ok())) {
    if (Date.now() > deadline) throw new Error(`Settings tab ${tab} is not the only one selected: ${JSON.stringify(seen)}`);
    await new Promise((r) => setTimeout(r, 250));
  }
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
    // A box stays disabled until its stored value loads (NotificationsTab.tsx).
    await box.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: `"${text}" stayed disabled` });
    const before = await box.isSelected();
    await box.click();
    await resolveIntent(b, { passphrase, appearMs: 1500 });
    await waitUntil(async () => (await (await optionBox(b, text)).isSelected()) !== before, WAIT_MS, `"${text}" did not flip`);
    await closeSettings(b);
    await openSettings(b, tab);
    // Read the stored value, not the box's initial one: every box is
    // disabled and unticked until its value loads (NotificationsTab.tsx,
    // PrivacyTab.tsx); ST21 and ST28 read an initial value back in walks 11-12.
    const reopened = await optionBox(b, text);
    await reopened.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: `"${text}" stayed disabled after reopening` });
    const stored = await reopened.isSelected();
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

/**
 * Wait until the stored preference `key` reads `value`. A `<select>` shows
 * the choice at once, but `handleSaveSettings` (settings.actions.ts:30)
 * first awaits `get_preferences` and only then sends `set_preferences`:
 * closing Settings before that kills the save (ST8). The read is started
 * by one script and collected by later ones (async scripts timed out).
 */
export async function waitStored(b: Browser, key: string, value: unknown): Promise<void> {
  await waitUntil(
    async () => {
      const got = await b.execute((k: string) => {
        const w = window as unknown as { __walkPref?: unknown; __TAURI_INTERNALS__: { invoke(cmd: string): Promise<Record<string, unknown>> } };
        const prev = w.__walkPref;
        w.__TAURI_INTERNALS__.invoke("get_preferences").then((p) => { w.__walkPref = p[k]; });
        return prev;
      }, key);
      return got === value;
    },
    WAIT_MS,
    `stored ${key} never read ${String(value)}`,
  );
}

/** Open the Add Friend modal from the buddy list (left open when already shown). */
export async function openAddFriend(b: Browser): Promise<void> {
  if (!(await present(b, ".form-tabs-equal")) && (await underTree(b))) {
    // The pack's Add Friends button follows the Host's AddFriend
    // capability; the first menu's "Add Friend…" is always there.
    await menu(b, "Rekindle", "Add Friend…");
    await shown(b, ".form-tabs-equal");
  } else if (!(await present(b, ".form-tabs-equal"))) {
    // The bar shows Add Friend on the Friends tab only (BottomActionBar.tsx:
    // 34-44); the community rows leave the Communities tab active (B57's
    // failure in walk 3).
    await (await shown(b, '.buddy-tab[title^="Friends"]')).click();
    await (await shown(b, '[aria-label="Add Friend"]')).click();
    await shown(b, ".form-tabs-equal");
  }
}

/** Submit an invite in the open Join Community modal (JoinCommunityModal.tsx). */
export async function submitJoin(b: Browser, link: string, passphrase: string): Promise<void> {
  await typeInto(await shown(b, 'input[placeholder="rekindle://invite/..."]'), link);
  await (await b.$('.modal-container button[type="submit"]')).click();
  await resolveIntent(b, { passphrase, appearMs: 3000 });
  // A refused join keeps the modal open with the Host's reason in
  // `.form-error` (SimpleInputModal.tsx:47-49, lifecycle.ts:60-68): report it.
  let refused = "";
  await waitUntil(
    async () => {
      const err = await b.$(".modal-container .form-error");
      if ((await err.isExisting()) && (await err.isDisplayed())) refused = await err.getText();
      return refused !== "" || !(await present(b, 'input[placeholder="rekindle://invite/..."]'));
    },
    NET_WAIT_MS,
    'input[placeholder="rekindle://invite/..."] still displayed after the join, with no error',
  );
  if (refused) throw new Error(`join refused: ${refused}`);
}
