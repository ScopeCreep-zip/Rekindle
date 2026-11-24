// Plan E7.14: the skin journeys the real-app specs share (applying,
// importing and keeping a skin, the login skin) and moving about a skin's
// tree (its views, the controls that show them); split from journeys.ts,
// which re-exports them.

import type { Browser } from "webdriverio";
import { NET_WAIT_MS, byText, gone, press, resolveIntent, shown, sleep, switchTo, waitForHandle, waitUntil } from "../walk/helpers.ts";
import { BUDDY, SETTINGS, pickNextFile } from "../walk/nav.ts";
import type { User } from "./user.ts";

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
  const linked = async (): Promise<string> => (await b.execute(LINKED_SHEET)) as string;
  // The sheet shown before the change: the new one links only after every
  // image it lists has decoded (useSkin.ts), seconds after the row says
  // "in use", so any skin:// sheet before then is the old skin's.
  let before = "";
  if ((await inUse()) !== name) {
    // A window just opened links its current sheet only once its images
    // decode, so wait for it: read early, `before` was "" and the old skin's
    // sheet, linking late, passed as the new one (skin-login on c5802aa26,
    // both tests one skin behind after a fresh Settings window).
    await waitUntil(async () => (before = await linked()).startsWith("skin://"), NET_WAIT_MS, "Settings never linked its current skin's sheet");
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
      href = await linked();
      return href.startsWith("skin://") && href !== before;
    },
    NET_WAIT_MS,
    `Settings never linked ${name}'s sheet`,
  );
  return href;
}

/**
 * Import the package `bytes` (a derived pack, skin-pack.ts `derivedPack`)
 * through Settings → Appearance → Import skin, choose it (its trial) and keep
 * it; the href of its sheet, as `applySkin` answers it.
 */
export async function importAndKeepSkin(b: Browser, user: User, bytes: Buffer, name: string): Promise<string> {
  await switchTo(b, SETTINGS);
  await (await byText(b, ".form-tab", "Appearance")).click();
  await pickNextFile(b, `${name}.rkskin`, "application/octet-stream", bytes.toString("base64"));
  await (await byText(b, "button.form-btn-secondary", "Import skin")).click();
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 3000 });
  const listed = async (): Promise<boolean> =>
    (await b.execute((n: string) => Array.from(document.querySelectorAll(".skin-row .skin-name")).some((e) => e.textContent?.trim() === n), name)) as boolean;
  await waitUntil(
    async () => {
      const error = await b.$(".skin-error");
      if ((await error.isExisting()) && (await error.isDisplayed())) throw new Error(`the import was refused: ${await error.getText()}`);
      return listed();
    },
    NET_WAIT_MS,
    `${name} was never listed`,
  );
  const href = await applySkin(b, user, name);
  // A library skin is on trial until kept (SkinTrialPrompt).
  await press(b, await shown(b, ".skin-trial .sec-btn-commit", NET_WAIT_MS));
  await resolveIntent(b, { passphrase: user.passphrase, appearMs: 1500 });
  await gone(b, ".skin-trial", NET_WAIT_MS);
  return href;
}

/** The href of the window's linked, loaded skin sheet (useSkin.ts LINK_ID), or "". */
export const LINKED_SHEET = 'var l = document.getElementById("rekindle-skin"); return l && l.sheet ? l.getAttribute("href") : "";';

/**
 * Wait until the window `b` is in shows the skin sheet `href` (applySkin's
 * answer). Each window links a new skin itself, after decoding its own
 * trees' art (plan SK4), and swaps its tree with that link (useSkin
 * setSkinShells), so until then it still draws the previous skin's tree.
 */
export async function skinShown(b: Browser, href: string): Promise<void> {
  let now = "";
  await waitUntil(
    async () => {
      now = (await b.execute(LINKED_SHEET)) as string;
      return now === href;
    },
    NET_WAIT_MS,
    `the window never showed the applied skin (shows ${now || "no sheet"})`,
  );
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

/** Whether the skin tree's `view` node is shown (not under a hidden page, with a box). */
export async function skinViewShown(b: Browser, view: string): Promise<boolean> {
  return (await b.execute((v: string) => {
    const n = document.querySelector<HTMLElement>(`[data-skin-role="view:${v}"]`);
    return !!n && n.closest("[hidden]") === null && n.getBoundingClientRect().height > 0;
  }, view)) as boolean;
}

/** The first visible skin node matching `selector` in `b`'s window, or "". */
async function visibleSkinNode(b: Browser, selector: string): Promise<string> {
  return (await b.execute(
    (sel: string) =>
      Array.from(document.querySelectorAll<HTMLElement>(sel)).find((n) => n.getBoundingClientRect().width > 0)?.dataset.skinNode ?? "",
    selector,
  )) as string;
}

/**
 * The node a press on control `id` lands on: the control when it listens
 * for presses, else its first shown descendant that does (Xfire Classic's
 * tab is a text node over its button Part, which carries the press). A user
 * presses what they see; this presses what listens there.
 */
async function pressNode(b: Browser, id: string): Promise<string> {
  return (await b.execute((control: string) => {
    const pressable = (n: Element): boolean => /\b(leftDown|click)\b/.test(n.getAttribute("data-skin-on") ?? "");
    const root = document.querySelector(`[data-skin-node="${control}"]`);
    if (!root || pressable(root)) return control;
    const inner = Array.from(root.querySelectorAll<HTMLElement>("[data-skin-on]")).find(
      (n) => pressable(n) && n.getBoundingClientRect().width > 0,
    );
    return inner?.dataset.skinNode ?? control;
  }, id)) as string;
}

/**
 * Under a skin's tree, press the pack's control for `view` wherever the Host
 * placed it (plan SK2-N4) and wait for the view: a tab or knob node; a knob
 * whose stops share one box (Combat), pressed round to the view; or a control
 * in a pack popup (StealthX's MainMenuPopup rail), pressed in the popup.
 */
export async function showSkinView(b: Browser, view: string): Promise<void> {
  await switchTo(b, BUDDY);
  const control = await visibleSkinNode(b, `[data-skin-role="tab:${view}"], [data-skin-role="knob:${view}"]`);
  if (control) {
    await press(b, await b.$(`[data-skin-node="${await pressNode(b, control)}"]`));
  } else if (await visibleSkinNode(b, '[data-skin-role^="knob:"]')) {
    for (let i = 0; i < 4 && !(await skinViewShown(b, view)); i++) {
      await press(b, await b.$(`[data-skin-node="${await visibleSkinNode(b, '[data-skin-role^="knob:"]')}"]`));
      await sleep(600);
    }
  } else {
    const launcher = await visibleSkinNode(b, '[data-skin-action^="launchPopup"]');
    if (!launcher) throw new Error(`no control for ${view} in the main window, a knob or a popup`);
    // The launcher toggles its popup (Xfire's MENU, shell popup.rs
    // close_if_open), and a choice in the rail leaves it open: use the
    // popup already open, else open it.
    const before = await b.getWindowHandles();
    const open = before.find((h) => h.startsWith(`popup-${BUDDY}-`));
    if (open) {
      await b.switchToWindow(open);
    } else {
      await press(b, await b.$(`[data-skin-node="${launcher}"]`));
      await b.switchToWindow(await waitForHandle(b, (h) => h.startsWith("popup-") && !before.includes(h), 10_000));
    }
    // The rail's control, by the node that listens for its press (as in the
    // main window: a control may carry its press on a child Part).
    const item = await shown(b, `[data-skin-role="tab:${view}"], [data-skin-role="knob:${view}"]`);
    const id = (await item.getAttribute("data-skin-node")) ?? "";
    await press(b, await b.$(`[data-skin-node="${await pressNode(b, id)}"]`));
    await switchTo(b, BUDDY);
  }
  await waitUntil(() => skinViewShown(b, view), 10_000, `the ${view} view did not show`);
}
