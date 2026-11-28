// The page's console, recorded for a spec to assert on (moved from
// journeys.ts by topic: it is no journey).

import type { Browser } from "webdriverio";

/**
 * From now on, record every `console.warn` and `console.error` the window
 * `b` is on writes (they still print), so a spec can assert one never
 * happened; `consoleSeen` reads them. Lasts until the page reloads.
 */
export async function watchConsole(b: Browser): Promise<void> {
  await b.execute(`if (!window.__rekindleConsole) {
  window.__rekindleConsole = [];
  for (const level of ["warn", "error"]) {
    const original = console[level].bind(console);
    console[level] = (...args) => {
      window.__rekindleConsole.push(level + ": " + args.map((a) => (a instanceof Error ? a.message : typeof a === "string" ? a : JSON.stringify(a))).join(" "));
      original(...args);
    };
  }
}`);
}

/** What `watchConsole` recorded in the window `b` is on. */
export async function consoleSeen(b: Browser): Promise<string[]> {
  return (await b.execute("return window.__rekindleConsole || null;")) as string[];
}
