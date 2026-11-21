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

/** OwnRecordsWait.tsx's text while our identity's records are pending (plan E2.5 FC3). */
const PUBLISHING = "Publishing your identity…";
/** How OwnRecordsWait.tsx's alert starts when publishing failed. */
const PUBLISH_FAILED = "Your identity could not be published";

/**
 * The own-records wait this window shows (FC3, OwnRecordsWait.tsx, and the
 * Public Key tab's own "Publishing your identity…" before its code): the
 * failure's text if publishing failed, else the wait's text while it
 * shows, else "".
 */
async function ownRecordsNote(b: Browser): Promise<string> {
  return (await b.execute(
    (waitText: string, failed: string) => {
      const visible = (e: Element) => (e as HTMLElement).offsetParent !== null;
      const text = (e: Element) => (e.textContent ?? "").replace(/\s+/g, " ").trim();
      const alert = Array.from(document.querySelectorAll('[role="alert"]')).find((e) => visible(e) && text(e).startsWith(failed));
      if (alert) return text(alert);
      const wait = Array.from(document.querySelectorAll('[role="status"]')).find((e) => visible(e) && text(e).startsWith(waitText));
      return wait ? text(wait) : "";
    },
    PUBLISHING,
    PUBLISH_FAILED,
  )) as string;
}

/**
 * Wait until this window no longer says our identity is being published
 * (FC3: records are held ~15–25 s after a fresh start). A failed publish
 * fails at once with its reason; a wait that outlasts the network bound
 * fails quoting what the window still says. `what` names the step.
 */
export async function untilPublished(b: Browser, what: string): Promise<void> {
  const deadline = Date.now() + NET_WAIT_MS;
  for (;;) {
    const note = await ownRecordsNote(b);
    if (note.startsWith(PUBLISH_FAILED)) throw new Error(`${what}: ${note}`);
    if (note === "") return;
    if (Date.now() >= deadline) throw new Error(`${what}: after ${NET_WAIT_MS / 1000} s the window still says "${note}"`);
    await sleep(250);
  }
}

export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export const MOD_KEY = process.platform === "darwin" ? Key.Command : Key.Ctrl;
export { Key };

/**
 * The first displayed match of `selector`, once one is. Reads every match,
 * as `present()` does: webdriverio 10's `$` throws on a second match (walk
 * 20: `.skin-row` matched every listed skin).
 */
export async function shown(b: Browser, selector: string, timeout = WAIT_MS): Promise<El> {
  const deadline = Date.now() + timeout;
  while (true) {
    for (const el of await b.$$(selector)) {
      if (await el.isDisplayed()) return el as unknown as El;
    }
    if (Date.now() >= deadline) throw new Error(`${selector} not displayed after ${timeout} ms`);
    await sleep(100);
  }
}

/** Once no match of `selector` is displayed (every match read, as `shown`). */
export async function gone(b: Browser, selector: string, timeout = WAIT_MS): Promise<void> {
  await waitUntil(async () => !(await present(b, selector)), timeout, `${selector} still displayed after ${timeout} ms`);
}

/**
 * Whether any match of `selector` is displayed. webdriverio 10's `$` is
 * strict and throws on a second match (walk 18: B61 once the peer joined the
 * member list, two `.member-item`s), so this reads every match.
 */
export async function present(b: Browser, selector: string): Promise<boolean> {
  for (const el of await b.$$(selector)) if (await el.isDisplayed()) return true;
  return false;
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
  // A chat composer says "Publishing your identity…" above it while our
  // records are pending (FC3): send once they are held.
  await untilPublished(b, selector);
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
  // A committed invite, friend request or accept is held in the dialog,
  // saying "Publishing your identity…", until our records are (FC3); the
  // dialog then closes, or names a failed publish.
  const closed = async () => !(await b.getWindowHandles()).includes(here) || !(await (await b.$(".intent-dialog")).isDisplayed());
  const deadline = Date.now() + NET_WAIT_MS;
  for (;;) {
    if (await closed().catch(() => false)) return "committed";
    const note = await ownRecordsNote(b).catch(() => "");
    if (note.startsWith(PUBLISH_FAILED)) throw new Error(`intent dialog after commit: ${note}`);
    if (Date.now() >= deadline) throw new Error(`intent dialog stayed open after commit${note ? ` saying "${note}"` : ""}`);
    await sleep(250);
  }
}

/** `work`, or a rejection with `msg` after `ms` (the work itself is not cancelled). */
export function within<T>(work: Promise<T>, ms: number, msg: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(msg)), ms);
  });
  return Promise.race([work, deadline]).finally(() => clearTimeout(timer));
}

/** The identity bar's name field, focus, label and intent dialog, read in
 *  the page: B12 names them on a failure (it failed 3 of 7 walks). */
export function identityBarState(b: Browser): Promise<string> {
  return b.execute(() => {
    const f = document.querySelector<HTMLInputElement>(".identity-bar-edit");
    const label = document.querySelector(".identity-bar-name")?.textContent ?? "none";
    const focus = document.activeElement?.className || document.activeElement?.tagName || "none";
    return `field=${f ? JSON.stringify(f.value) : "gone"} focus=${focus} label=${JSON.stringify(label)} dialog=${!!document.querySelector(".intent-dialog")}`;
  });
}
