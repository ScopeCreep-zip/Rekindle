// Probe (lead, 2026-11-26): a selection made in Settings → Profile's
// Display Name collapses to a caret about a second after select() (Linux's
// OS capture: selectionStart = selectionEnd = 3). This records, in the page,
// everything that could collapse it, each with its time after select():
// every write to the field's value (and the stack that made it), each
// change of <html data-window-focus>, focus and blur on the window and the
// field, every selectionchange, and the selection itself every 100 ms for
// three seconds. The value trap is the spec's own, set on this one element.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown } from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const FIELD = '.settings-content input.form-input[type="text"]';

describe("selection collapse probe (Settings → Profile, Display Name)", function () {
  this.timeout(300_000);
  const me: User = { ...newUser("collapse"), displayName: "Ari" };

  it("logs what happens to a fresh selection over three seconds", async () => {
    await createIdentity(browser, me);
    // settings-theme selects right after switching to Profile; a settled
    // window kept the selection (98c48fd04's run), so the probe now does the
    // same: another tab first, then Profile, then select at once.
    await openSettings(browser, "Application");
    await press(browser, await byText(browser, ".form-tab", "Profile"));
    await shown(browser, FIELD, NET_WAIT_MS);
    await browser.execute((sel: string) => {
      const f = document.querySelector<HTMLInputElement>(sel);
      if (!f) return;
      const log: string[] = [];
      const t0 = { at: 0 };
      const now = (): string => `${Math.round(performance.now() - t0.at)}ms`;
      (window as unknown as { __collapse: { log: string[]; t0: { at: number } } }).__collapse = { log, t0 };
      // Writes to this field's value, with the caller.
      const proto = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      if (proto?.set && proto.get) {
        const set = proto.set;
        const get = proto.get;
        Object.defineProperty(f, "value", {
          configurable: true,
          get() { return get.call(this); },
          set(v: string) {
            log.push(`${now()} value set "${v}" (was "${get.call(this)}") at ${(new Error().stack ?? "").split("\n").slice(2, 6).join(" | ")}`);
            set.call(this, v);
          },
        });
      }
      new MutationObserver((ms) => {
        for (const m of ms) log.push(`${now()} html ${m.attributeName}=${document.documentElement.getAttribute(m.attributeName ?? "")}`);
      }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-window-focus"] });
      window.addEventListener("focus", () => log.push(`${now()} window focus`));
      window.addEventListener("blur", () => log.push(`${now()} window blur`));
      f.addEventListener("focus", () => log.push(`${now()} field focus`));
      f.addEventListener("blur", () => log.push(`${now()} field blur`));
      document.addEventListener("selectionchange", () => log.push(`${now()} selectionchange ${f.selectionStart}-${f.selectionEnd}`));
      t0.at = performance.now();
      f.focus();
      f.select();
      log.push(`${now()} selected ${f.selectionStart}-${f.selectionEnd} of ${f.value.length}, focused ${document.activeElement === f}, hasFocus ${document.hasFocus()}`);
      let ticks = 0;
      const poll = setInterval(() => {
        const current = document.querySelector<HTMLInputElement>(sel);
        const same = current === f;
        log.push(`${now()} poll ${f.selectionStart}-${f.selectionEnd} active ${document.activeElement === f} same-element ${same}${same ? "" : ` (new element ${current?.selectionStart}-${current?.selectionEnd}, active ${document.activeElement === current})`}`);
        if (++ticks >= 30) clearInterval(poll);
      }, 100);
    }, FIELD);
    await browser.pause(3500);
    await shot("after-three-seconds");
    const log = (await browser.execute(() => (window as unknown as { __collapse?: { log: string[] } }).__collapse?.log ?? [])) as string[];
    // Only the changes: polls that repeat the last one are dropped.
    let last = "";
    for (const line of log) {
      const key = line.replace(/^\d+ms /, "");
      if (key.startsWith("poll") && key === last) continue;
      last = key;
      console.log(`[collapse] ${line.slice(0, 400)}`);
    }
  });
});
