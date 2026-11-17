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

/** First displayed element under `selector` whose visible text contains `text`. */
export async function byText(b: Browser, selector: string, text: string, timeout = WAIT_MS): Promise<El> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
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
  await el.scrollIntoView({ block: "center" });
  await b.execute((node: HTMLElement) => {
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
