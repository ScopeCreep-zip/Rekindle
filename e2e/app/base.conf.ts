// Plan E7.14: what every real-app config shares. A WebdriverIO config holds
// either standalone capabilities (wdio.conf.ts, one app per worker) or one
// multiremote set (wdio.peers.conf.ts, an app per named user), never both
// (webdriver.io/docs/multiremote), so the suite has one config per shape.

import { mkdirSync } from "node:fs";
import path from "node:path";
import { browser } from "@wdio/globals";
import type { Browser } from "webdriverio";
import { keepDir } from "./keep.ts";
import { RekindleAppLauncher } from "./launcher.ts";
import { timingAfter, timingBefore, timingTest } from "./timing.ts";

/** The driver's script bound per session (see `before`). */
const SCRIPT_TIMEOUT_MS = 90_000;

/** Failure evidence, in the run's folder beside its app logs (keep.ts). */
const KEEP_DIR = keepDir();

/** Every user's session: one standalone, or each multiremote instance by name. */
function sessions(): [string, Browser][] {
  if (!browser.isMultiRemote) return [["app", browser as unknown as Browser]];
  const multi = browser as unknown as WebdriverIO.MultiRemoteBrowser;
  return multi.instances.map((name) => [name, multi.getInstance(name)]);
}

/**
 * On a failed test, each user's current window as a screenshot and the
 * windows open in its app, in `KEEP_DIR`: the evidence of what the user
 * saw, as the walk's per-row screenshot was.
 */
async function recordFailure(title: string): Promise<void> {
  mkdirSync(KEEP_DIR, { recursive: true });
  const stem = title.replace(/[^\w-]+/g, "_").slice(0, 80);
  for (const [name, b] of sessions()) {
    const handles = await b.getWindowHandles().catch(() => [] as string[]);
    const current = await b.getWindowHandle().catch(() => "none");
    console.log(`[failure] ${name}: window ${current} of [${handles.join(", ")}]`);
    await b.saveScreenshot(path.join(KEEP_DIR, `${stem}-${name}.png`)).catch(() => undefined);
  }
}

export const base = {
  runner: "local",
  // One worker at a time: real apps share this machine's CPU, and each is
  // its own Veilid node.
  maxInstances: 1,
  services: [[RekindleAppLauncher, {}]],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 300_000 },
  reporters: ["spec"],
  // `warn`: `info` logs every command's body (typed passphrases included);
  // request headers are never logged at any level.
  logLevel: "warn",
  waitforTimeout: 10_000,
  connectionRetryCount: 1,
  // The driver's bound on one script, screenshot or element command
  // (tauri-plugin-wdio-webdriver: 30 s by default, set per session by W3C
  // Set Timeouts; New Session ignores a `timeouts` capability). Sized from
  // the timing run under load (2026-11-21, load 56.8): the slowest command,
  // an axe/geometry execute, took 29.2 s, and round 15 timed out on such
  // calls three times. 90 s is about three times that, and stays under
  // webdriverio's 120 s `connectionRetryTimeout`, so the driver's own timeout
  // is the one reported. A command past the old 30 s still prints the
  // processes (timing.ts), so a stall this absorbs is not hidden.
  async before() {
    for (const [name, b] of sessions()) {
      await b.setTimeout({ script: SCRIPT_TIMEOUT_MS });
      const { script } = await b.getTimeouts();
      if (script !== SCRIPT_TIMEOUT_MS) throw new Error(`${name}: the driver kept a script timeout of ${script} ms`);
    }
  },
  // Each command's duration, printed when slow or timed out (timing.ts).
  beforeTest(test: { title: string }) {
    timingTest(test.title);
  },
  beforeCommand(commandName: string) {
    timingBefore(commandName);
  },
  afterCommand(commandName: string, _args: unknown[], _result: unknown, error?: Error) {
    timingAfter(commandName, error);
  },
  async afterTest(test: { title: string }, _context: unknown, result: { passed: boolean }) {
    if (!result.passed) await recordFailure(test.title);
  },
} satisfies Partial<WebdriverIO.Config>;
