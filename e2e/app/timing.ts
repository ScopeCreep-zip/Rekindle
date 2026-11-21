// Plan E7.14 (round 15's load failures, 2026-11-21): how long each WebDriver
// command takes, so the driver's script timeout is sized from evidence and
// a stall on a trivial call is told apart from heavy work. Round 15 timed
// out twice on heavy work (skins-a11y: the axe/geometry execute and the
// snapshot) and once on a trivial read (layout-friends: `element/<id>/enabled`
// under a waitUntil), each past the driver's 30 s script bound
// (tauri-plugin-wdio-webdriver webdriver/session.rs `Timeouts::default`).
//
// Every command that takes `SLOW_MS` or more prints one `[timing]` line: the
// test, the command's name and milliseconds, and the machine's load average.
// A command that timed out also prints the busiest processes and the WebKit
// and app processes' CPU and state: what the machine was doing while the page
// gave no answer. Names only: a command's arguments can hold a typed
// passphrase and are never printed.

import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

/** Commands at or past this many milliseconds are printed. */
const SLOW_MS = 2_000;
/**
 * The driver's default script bound, which round 15's stalls passed: a
 * command this slow prints the processes even now that the bound is larger
 * (base.conf.ts), so a stall the larger bound absorbs is still seen.
 */
const STALL_MS = 30_000;
/** Wrappers that wait by design (they poll other commands, which are timed themselves). */
const WAITS = /^(waitUntil|waitFor|pause$)/;

const started = new Map<string, number[]>();
let test = "(before any test)";

/** The test now running, for the lines that follow. */
export function timingTest(title: string): void {
  test = title;
}

export function timingBefore(command: string): void {
  const stack = started.get(command) ?? [];
  stack.push(Date.now());
  started.set(command, stack);
}

export function timingAfter(command: string, error: unknown): void {
  const at = started.get(command)?.pop();
  if (at === undefined || WAITS.test(command)) return;
  const ms = Date.now() - at;
  const timedOut = error instanceof Error && /timed out/i.test(error.message);
  if (ms < SLOW_MS && !timedOut) return;
  const load = os.loadavg().map((n) => n.toFixed(1)).join(" ");
  const stalled = ms >= STALL_MS;
  console.log(`[timing] ${test} | ${command} ${ms} ms | load ${load}${timedOut ? " | TIMED OUT" : stalled ? " | PAST 30 s" : ""}`);
  if (timedOut || stalled) console.log(`[timing] processes: ${processes()}`);
}

/** The six busiest processes, then every WebKit and app process: CPU %, state, name. */
function processes(): string {
  const sort = process.platform === "darwin" ? ["-r"] : ["--sort=-pcpu"];
  let out: string;
  try {
    out = execFileSync("ps", ["-Ao", "pcpu=,state=,comm=", ...sort], { encoding: "utf8", timeout: 5_000 });
  } catch (err) {
    return `ps failed: ${err instanceof Error ? err.message : String(err)}`;
  }
  const rows = out
    .split("\n")
    .map((l) => l.trim().split(/\s+/))
    .filter((f) => f.length >= 3)
    .map(([cpu, state, ...comm]) => ({ cpu, state, name: path.basename(comm.join(" ")) }));
  const watched = /WebContent|WebKitWebProcess|WebKitNetworkProcess|GPU|rekindle/;
  const picked = [...rows.slice(0, 6), ...rows.slice(6).filter((r) => watched.test(r.name))];
  return picked.map((r) => `${r.name} ${r.cpu}% ${r.state}`).join("; ");
}
