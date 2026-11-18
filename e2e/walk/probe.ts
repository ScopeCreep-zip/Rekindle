// In-page probes a failing row names in its note: the IPC calls the page made
// and the events its window was sent, each with its time.
//
// Tauri defines `__TAURI_INTERNALS__.invoke` and `.ipc` read-only (tauri
// 2.12.1 scripts/core.js:81, ipc.js:142), so a call is seen at the `fetch` its
// IPC protocol posts it with (scripts/ipc-protocol.js:37). An event reaches the
// page through the window's `Channel`, whose callback lives in the
// `__TAURI_INTERNALS__.callbacks` map (core.js:66, exposed "for debugging
// purposes"); `runCallback` looks each one up there, so a wrapper put in its
// place sees every delivery.

import type { Browser } from "webdriverio";

/**
 * Start (or restart) recording: calls to any command in `cmds`, events whose
 * JSON contains any of `events`, and every Enter. Each entry is stamped with
 * the milliseconds since the recording started.
 */
export async function recordPage(b: Browser, cmds: string[], events: string[]): Promise<void> {
  await b.execute((cmdList: string[], eventList: string[]) => {
    type Cb = (data: unknown) => unknown;
    type W = Window & {
      __walkLog?: string[];
      __walkT0?: number;
      __walkCmds?: string[];
      __walkEvents?: string[];
      __walkFetch?: typeof fetch;
      __walkWrapped?: WeakSet<Cb>;
      __walkIndices?: Record<number, number[]>;
      __walkThrows?: string[];
      __TAURI_INTERNALS__?: { callbacks?: Map<number, Cb> };
    };
    const w = window as W;
    w.__walkLog = [];
    w.__walkT0 = Date.now();
    w.__walkCmds = cmdList;
    w.__walkEvents = eventList;
    const log = (line: string): number => w.__walkLog!.push(`+${Date.now() - w.__walkT0!}ms ${line}`) - 1;
    // Wrap every callback now registered; the window's Channel is one of them.
    const callbacks = w.__TAURI_INTERNALS__?.callbacks;
    w.__walkWrapped ??= new WeakSet();
    if (callbacks) {
      for (const [id, cb] of callbacks) {
        if (w.__walkWrapped.has(cb)) continue;
        const wrapped: Cb = (data) => {
          const text = JSON.stringify(data) ?? "";
          // Every Channel index this callback was handed, kept across
          // restarts: the Channel runs a message only once every lower index
          // has arrived (@tauri-apps/api 2.12.1 core.js:137-156).
          if (typeof data === "object" && data !== null && "index" in data) {
            ((w.__walkIndices ??= {})[id] ??= []).push((data as { index: number }).index);
          }
          if (w.__walkEvents!.some((e) => text.includes(e))) {
            log(`event ${text.slice(0, 400)}`);
          } else if (typeof data === "object" && data !== null && "index" in data) {
            // Every other Channel delivery by its index only: a gap holds
            // back every later message (@tauri-apps/api 2.12.1 core.js:137-156).
            const m = (data as { index: number; message?: { channel?: string; cursor?: { seq?: number } | null } });
            log(`delivery #${m.index} ${m.message?.channel ?? "end"} seq=${m.message?.cursor?.seq ?? "-"} ${text.length}B`);
          }
          // Whether the page's handler ran: it stores the cursor of every
          // journaled event it processes (subscriptions.ts:137-139).
          const before = sessionStorage.getItem("rekindle.eventCursor");
          let out: unknown;
          try {
            out = cb(data);
          } catch (err) {
            // Kept across restarts: a throw inside the Channel's handler
            // leaves its next index unadvanced (core.js:137-140).
            (w.__walkThrows ??= []).push(`#${(data as { index?: number }).index} threw ${String(err)} at ${String((err as Error)?.stack ?? "").split("\n").slice(0, 4).join(" < ")} on ${text.slice(0, 500)}`);
            throw err;
          }
          if (text.includes('"event-stream"')) log(`stream frame ${text.slice(0, 200)}`);
          if (text.includes('"cursor":{') && sessionStorage.getItem("rekindle.eventCursor") === before) {
            log(`#${(data as { index?: number }).index} not processed (cursor still ${before})`);
          }
          return out;
        };
        w.__walkWrapped.add(wrapped);
        callbacks.set(id, wrapped);
      }
    }
    if (w.__walkFetch) return;
    // Where the page's event stream stands: every cursor it stores
    // (subscriptions.ts writeCursor runs just before it dispatches an event),
    // and every error the page raises or logs.
    const realSet = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key: string, value: string) {
      if (key === "rekindle.eventCursor") log(`cursor stored ${value}`);
      return realSet.call(this, key, value);
    };
    window.addEventListener("error", (e) => log(`error ${String(e.message)} at ${e.filename}:${e.lineno}`));
    window.addEventListener("unhandledrejection", (e) => log(`unhandled rejection ${String((e.reason as Error)?.stack ?? e.reason).slice(0, 400)}`));
    const realError = console.error.bind(console);
    console.error = (...args: unknown[]) => {
      log(`console.error ${args.map((a) => (a instanceof Error ? a.stack ?? a.message : String(a))).join(" ").slice(0, 400)}`);
      realError(...args);
    };
    document.addEventListener("keydown", (e) => {
      if (e.key !== "Enter") return;
      const t = e.target as HTMLInputElement;
      log(`Enter on ${t.className || t.tagName} value=${JSON.stringify(t.value ?? null)}`);
    }, true);
    window.addEventListener("keydown", (e) => {
      if (e.key === "Enter") log(`Enter prevented=${e.defaultPrevented}`);
    });
    const real = window.fetch.bind(window);
    w.__walkFetch = real;
    window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const p = real(input, init);
      // A Channel message of 8 KiB or more is not evaluated inline: the page
      // fetches it with this command, then runs the callback (tauri 2.12.1
      // ipc/channel.rs:295-317).
      const cmd = url.includes("__TAURI_CHANNEL__") ? "channel-fetch" : w.__walkCmds!.find((c) => url.endsWith(`/${c}`));
      if (cmd) {
        const at = log(`sent ${cmd} ${String(init?.body).slice(0, 300)}`);
        const t0 = Date.now();
        p.then(
          (r) => r.clone().text().then((t) => {
            w.__walkLog![at] += ` -> after ${Date.now() - t0}ms ${r.status} ${r.headers.get("Tauri-Response") ?? "?"} ${t.slice(0, 300)}`;
          }),
          (e: unknown) => { w.__walkLog![at] += ` -> fetch error ${String(e)}`; },
        );
      }
      return p;
    };
  }, cmds, events);
}

/** What the recording holds, one entry per call, event or Enter. */
export async function pageLog(b: Browser): Promise<string> {
  const log = await b.execute(() => (window as Window & { __walkLog?: string[] }).__walkLog ?? []);
  const cursor = await b.execute(() => sessionStorage.getItem("rekindle.eventCursor"));
  // Per callback: the indices seen since the first recording, and the gaps
  // below the highest (a gap holds back every later message).
  const indices = await b.execute(() => {
    const seen = (window as Window & { __walkIndices?: Record<number, number[]> }).__walkIndices ?? {};
    return Object.entries(seen).map(([id, list]) => {
      const max = Math.max(...list);
      const missing = [...Array(max + 1).keys()].filter((i) => !list.includes(i));
      return `cb ${id}: ${list.length} seen, first ${list[0]}, max ${max}, missing [${missing.join(",")}]`;
    }).join(" | ");
  });
  const page = await b.execute(() => {
    const nav = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
    return `${nav?.type ?? "?"} at ${new Date(performance.timeOrigin).toISOString()}`;
  });
  // The summary first: a row's note is cut at 2000 characters (walk.ts).
  const throws = await b.execute(() => (window as Window & { __walkThrows?: string[] }).__walkThrows ?? []);
  return `page loaded by ${page} ; handler throws: ${throws.join(" | ") || "none"} ; cursor now ${cursor} ; indices ${indices || "none"} ; ${log.length ? log.join(" ; ") : "nothing recorded"}`;
}
