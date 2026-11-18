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
          if (w.__walkEvents!.some((e) => text.includes(e))) {
            log(`event ${text.slice(0, 400)}`);
          } else if (typeof data === "object" && data !== null && "index" in data) {
            // Every other Channel delivery by its index only: a gap holds
            // back every later message (@tauri-apps/api 2.12.1 core.js:137-156).
            const m = (data as { index: number; message?: { channel?: string; cursor?: { seq?: number } | null } });
            log(`delivery #${m.index} ${m.message?.channel ?? "end"} seq=${m.message?.cursor?.seq ?? "-"}`);
          }
          return cb(data);
        };
        w.__walkWrapped.add(wrapped);
        callbacks.set(id, wrapped);
      }
    }
    if (w.__walkFetch) return;
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
      const cmd = w.__walkCmds!.find((c) => url.endsWith(`/${c}`));
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
  return log.length ? log.join(" ; ") : "nothing recorded";
}
