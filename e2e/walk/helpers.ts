// Plan E7.13: driving helpers for the walk. The embedded server's element
// click is `el.click()` (platform/exec_element.rs), which never fires
// pointer events, so Kobalte triggers and menu items (opened and selected
// on pointerdown/pointerup) are pressed through W3C pointer actions
// instead (platform/pointer_input.rs synthesises the full sequence).

import { Key, type Browser } from "webdriverio";

type El = Awaited<ReturnType<Browser["$"]>>;

export const WAIT_MS = 10_000;
export const NET_WAIT_MS = 120_000;

export class Skip extends Error {}

export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export const MOD_KEY = process.platform === "darwin" ? Key.Command : Key.Ctrl;
export { Key };

export async function shown(b: Browser, selector: string, timeout = WAIT_MS): Promise<El> {
  const el = await b.$(selector);
  await el.waitForDisplayed({ timeout, timeoutMsg: `${selector} not displayed after ${timeout} ms` });
  return el;
}

export async function gone(b: Browser, selector: string, timeout = WAIT_MS): Promise<void> {
  const el = await b.$(selector);
  await el.waitForDisplayed({ timeout, reverse: true, timeoutMsg: `${selector} still displayed after ${timeout} ms` });
}

export async function present(b: Browser, selector: string): Promise<boolean> {
  const el = await b.$(selector);
  return (await el.isExisting()) && (await el.isDisplayed());
}

/**
 * Run every finite CSS animation in the page to its end state, as
 * Playwright's `animations: "disabled"` does. The walk's windows sit behind
 * other windows, where WebKit reports `visibilityState` "hidden" and its
 * animation clock stands still: a new message's `fade-in`
 * (animations.css:35-37, MessageBubble.tsx:136) stayed at currentTime 0 and
 * opacity 0, so webdriverio's isDisplayed (opacity checked) said false for a
 * message in the list (probe of M42, 2026-11-17). A window a user looks at
 * runs the 0-to-1 fade itself.
 */
export async function settleAnimations(b: Browser): Promise<void> {
  await b.execute(() => {
    for (const a of document.getAnimations()) {
      if (a.effect?.getComputedTiming().iterations !== Infinity) a.finish();
    }
  });
}

/** First displayed element under `selector` whose visible text contains `text`. */
export async function byText(b: Browser, selector: string, text: string, timeout = WAIT_MS): Promise<El> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    await settleAnimations(b);
    for (const el of await b.$$(selector)) {
      if ((await el.isDisplayed()) && (await el.getText()).includes(text)) return el as unknown as El;
    }
    await sleep(200);
  }
  throw new Error(`no ${selector} containing "${text}" after ${timeout} ms`);
}

/**
 * A pointer press dispatched in the page (pointerdown, mousedown, pointerup,
 * mouseup, click): Kobalte's menu triggers open on pointerdown, which the
 * server's element click does not send, and its W3C pointer actions timed
 * out as async scripts in the first walk (2026-11-17).
 */
export async function press(b: Browser, el: El): Promise<void> {
  // The page's own scrollIntoView, not webdriverio's: its version
  // (webdriverio 10.1.0 build/node.js:13730) waits for two stable
  // requestAnimationFrame callbacks in an async script, and a window behind
  // another (the buddy list while Settings is open) runs no frames, so every
  // press stalled to the 60 s script timeout ("execute/async" in walk 4).
  await b.execute((node: HTMLElement) => {
    node.scrollIntoView({ block: "center" });
    const r = node.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, composed: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2, button: 0 };
    const pointer = { ...at, pointerId: 1, pointerType: "mouse", isPrimary: true };
    node.dispatchEvent(new PointerEvent("pointerdown", { ...pointer, buttons: 1 }));
    node.dispatchEvent(new MouseEvent("mousedown", { ...at, buttons: 1 }));
    node.dispatchEvent(new PointerEvent("pointerup", pointer));
    node.dispatchEvent(new MouseEvent("mouseup", at));
    node.dispatchEvent(new MouseEvent("click", at));
  }, el as unknown as HTMLElement);
}

/** The server synthesises no `dblclick`, so it is dispatched in the page. */
export async function doubleClick(b: Browser, el: El): Promise<void> {
  await b.execute((node: HTMLElement) => {
    node.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, detail: 2 }));
  }, el as unknown as HTMLElement);
}

export async function typeInto(el: El, text: string): Promise<void> {
  await el.click();
  await el.clearValue();
  await el.addValue(text);
}

/**
 * Type `text` into a MessageInput textarea and send it with Enter, failing
 * at the step that stops: the text missing from the field after typing, or
 * the field still holding it after Enter (MessageInput.tsx onKeyDown clears
 * it once the send handler took the text).
 */
export async function typeAndSend(b: Browser, selector: string, text: string): Promise<void> {
  const field = await shown(b, selector);
  await typeInto(field, text);
  await waitUntil(async () => (await field.getValue()) === text, WAIT_MS, `${selector} does not hold the typed text`);
  await b.keys("Enter");
  await waitUntil(async () => (await field.getValue()) === "", WAIT_MS, `${selector} kept the text after Enter: the send handler did not take it`);
}

export async function waitUntil(cond: () => Promise<boolean>, timeout: number, msg: string): Promise<void> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await cond().catch(() => false)) return;
    await sleep(250);
  }
  throw new Error(msg);
}

export async function waitForHandle(b: Browser, match: (h: string) => boolean, timeout = WAIT_MS): Promise<string> {
  let found = "";
  await waitUntil(
    async () => {
      found = (await b.getWindowHandles()).find(match) ?? "";
      return found !== "";
    },
    timeout,
    `no window handle matched after ${timeout} ms`,
  );
  return found;
}

export async function switchTo(b: Browser, label: string, timeout = WAIT_MS): Promise<void> {
  await waitForHandle(b, (h) => h === label, timeout);
  await b.switchToWindow(label);
}

export type IntentOutcome = "none" | "committed" | "cancelled";

/**
 * ADR 0019: a reviewed command shows the `IntentDialog`
 * (components/common/IntentDialog.tsx). When it appears, either cancel it
 * or give the proof it asks for (acknowledge box, typed subject, or
 * passphrase) and commit.
 */
export async function resolveIntent(
  b: Browser,
  opts: { passphrase: string; cancel?: boolean; appearMs?: number },
): Promise<IntentOutcome> {
  const dialog = await b.$(".intent-dialog");
  try {
    await dialog.waitForDisplayed({ timeout: opts.appearMs ?? 3000 });
  } catch {
    return "none";
  }
  if (opts.cancel) {
    await (await b.$(".intent-dialog .sec-btn-secondary")).click();
    await gone(b, ".intent-dialog");
    return "cancelled";
  }
  const ack = await b.$(".intent-dialog .sec-review-ack input[type=checkbox]");
  if ((await ack.isExisting()) && !(await ack.isSelected())) await ack.click();
  const field = await b.$(".intent-dialog .intent-dialog-field");
  if (await field.isExisting()) {
    const input = await field.$("input");
    if ((await input.getAttribute("type")) === "password") {
      await typeInto(input, opts.passphrase);
    } else {
      const label = await (await field.$("span")).getText();
      const subject = /^Type (.*) to confirm$/.exec(label.trim())?.[1] ?? "";
      await typeInto(input, subject);
    }
  }
  const commit = await b.$(".intent-dialog .sec-btn-commit, .intent-dialog .sec-btn-commit-danger");
  await commit.waitForEnabled({ timeout: WAIT_MS, timeoutMsg: "intent commit button stayed disabled" });
  const here = await b.getWindowHandle();
  await commit.click();
  // A committed logout closes the window the dialog is in; polling it then
  // fails with "No window could be found" (the hang in walk 2, after B1).
  await waitUntil(
    async () => !(await b.getWindowHandles()).includes(here) || !(await (await b.$(".intent-dialog")).isDisplayed()),
    NET_WAIT_MS,
    "intent dialog stayed open after commit",
  );
  return "committed";
}

/** `work`, or a rejection with `msg` after `ms` (the work itself is not cancelled). */
export function within<T>(work: Promise<T>, ms: number, msg: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(msg)), ms);
  });
  return Promise.race([work, deadline]).finally(() => clearTimeout(timer));
}
