// Plan E7.14 (lead, 2026-11-26): the machine's state beside every real-app
// run, read-only. A locked session hides every window: WebKit then clamps a
// page's timers and no app can come to the front, so a timing or
// `visibility` assertion cannot hold, while screenshots still render. Each
// run's log records it (base.conf.ts `before`), and a spec that times a page
// asserts the page is shown first (key-status). Nothing is changed.

import { execFileSync } from "node:child_process";

function run(cmd: string, args: string[]): string {
  try {
    return execFileSync(cmd, args, { encoding: "utf8", timeout: 5000 });
  } catch (e) {
    return `(${cmd} failed: ${String(e).slice(0, 80)})`;
  }
}

/** macOS: the session lock (ioreg), the relevant power assertions, and
 *  the app LaunchServices has in front, and the app's own type. */
function darwin(): Record<string, unknown> {
  const locked = /CGSSessionScreenIsLocked<\/key>\s*<true\/>/.test(run("ioreg", ["-n", "Root", "-d1", "-a"]));
  const assertions = run("pmset", ["-g", "assertions"])
    .split("\n")
    .filter((l) => /^\s+(UserIsActive|PreventUserIdleSystemSleep|PreventUserIdleDisplaySleep|BackgroundTask)\s/.test(l))
    .map((l) => l.trim().replace(/\s+/g, " "))
    .join(", ");
  // One entry per app, each opening `N) "name" ASN:…`.
  const entries = run("lsappinfo", ["list"]).split(/\n(?=\s*\d+\) ")/);
  const front = entries.find((e) => e.includes("(in front)"))?.match(/"([^"]+)"/)?.[1] ?? "none";
  const app = entries.find((e) => /executable path="[^"]*rekindle-desktop"/.test(e)) ?? "";
  const appLine = app ? (app.match(/type="[^"]*"/)?.[0] ?? "listed") : "not listed";
  return { screenLocked: locked, front, assertions, app: appLine };
}

/** Linux: logind's lock hint for this session, and the session type. */
function linux(): Record<string, unknown> {
  const id = process.env.XDG_SESSION_ID;
  const hint = id ? run("loginctl", ["show-session", id, "-p", "LockedHint", "--value"]).trim() : "(no XDG_SESSION_ID)";
  return { screenLocked: hint === "yes" ? true : hint === "no" ? false : hint, session: process.env.XDG_SESSION_TYPE ?? "unknown" };
}

/** The machine's state now, as one JSON line. */
export function machineState(): string {
  const state = process.platform === "darwin" ? darwin() : process.platform === "linux" ? linux() : { platform: process.platform };
  return JSON.stringify(state);
}
