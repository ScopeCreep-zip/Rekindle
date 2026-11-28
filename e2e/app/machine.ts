// Plan E7.14 (lead, 2026-11-26): the machine's state beside every real-app
// run, read-only. A locked session hides every window: WebKit then clamps a
// page's timers and no app can come to the front, so a timing or
// `visibility` assertion cannot hold, while screenshots still render. Each
// run's log records it (base.conf.ts `before`), a spec that times a page
// asserts the page is shown first (key-status), and a locked session is
// refused before any spec and before each test (refuseLockedSession).
// Nothing on the machine is changed.

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
  const locked = /CGSSessionScreenIsLocked<\/key>\s*<true\/>/.test(
    run("ioreg", ["-n", "Root", "-d1", "-a"])
  );
  const assertions = run("pmset", ["-g", "assertions"])
    .split("\n")
    .filter((l) =>
      /^\s+(UserIsActive|PreventUserIdleSystemSleep|PreventUserIdleDisplaySleep|BackgroundTask)\s/.test(
        l
      )
    )
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
  const hint = id
    ? run("loginctl", ["show-session", id, "-p", "LockedHint", "--value"]).trim()
    : "(no XDG_SESSION_ID)";
  return {
    screenLocked: hint === "yes" ? true : hint === "no" ? false : hint,
    session: process.env.XDG_SESSION_TYPE ?? "unknown",
  };
}

/** The machine's state now. */
function machineFacts(): Record<string, unknown> {
  return process.platform === "darwin"
    ? darwin()
    : process.platform === "linux"
      ? linux()
      : { platform: process.platform };
}

/** The machine's state now, as one JSON line. */
export function machineState(): string {
  return JSON.stringify(machineFacts());
}

/**
 * Refuse a run on a locked session (lead, 2026-11-27): WebKit throttles an
 * occluded window's timers, image decoding and painting, so the run would
 * fail as a product fault it is not (#53's device-reenrol on 1f780e508: the
 * skin never re-linked after a reload behind the lock screen). Throws, with
 * the machine state, when the session reports itself locked.
 */
export function refuseLockedSession(when: string): void {
  const facts = machineFacts();
  if (facts.screenLocked === true) {
    throw new Error(
      `invalid run (${when}): the screen is locked; WebKit throttles occluded windows, so no result here is a proof. Unlock and rerun. ${JSON.stringify(facts)}`
    );
  }
}
