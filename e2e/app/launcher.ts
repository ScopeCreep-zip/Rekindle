// Plan E7.14 S1: the WebdriverIO launcher service that starts the real app
// for each worker. It replaces `@wdio/tauri-service`, which cannot drive this
// app: its embedded provider polls `/status` and posts `/wdio/eval` with no
// way to send a credential (@wdio/tauri-service 1.5.1 `pollWebDriverStatus`,
// `DirectEvalClient.eval`), and our driver refuses any request without the
// run's bearer token (crates/vendor/tauri-plugin-wdio-webdriver/src/server/
// auth.rs); and its `browser.tauri.*` API needs `tauri-plugin-wdio` and
// `window.__TAURI__` (its docs/plugin-setup.md: "required"), which E7.13
// rejected (any window could script any other, defeating the per-window
// capability split).
//
// A launcher service is WebdriverIO's documented extension point for this
// (webdriver.io/docs/customservices): `onWorkerStart` runs in the launcher
// before each worker forks and may change that worker's capabilities. Each
// worker gets its own app on its own temp data root, port and token; the
// connection options (`hostname`, `port`, `headers`) ride on the capability,
// which the runner merges into the session's options and strips from what
// it sends the driver (@wdio/runner `sanitizeCaps`). The capabilities reach
// the worker over the fork's IPC channel, never its argv
// (@wdio/local-runner `childProcess.send`), so the token is not visible in
// a process listing; it is never printed.

import { copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import type { Capabilities, Services } from "@wdio/types";
import { SevereServiceError } from "webdriverio";
import { build, launch, type App } from "../walk/launch.ts";
import { keepDir, pruneKept } from "./keep.ts";

/**
 * Stop `app` and keep its log in the run's folder (keep.ts), passing or
 * failing: the launcher reuses one log name per worker, so the next run
 * would overwrite the evidence of a failure that shows up only in a log
 * (finding C-COM1, 2026-11-19, lost its log that way).
 */
async function stopAndKeepLog(app: App, failed?: string): Promise<void> {
  await app.stop(failed === undefined ? undefined : path.join(failed, path.basename(app.log, ".log")));
  const logs = path.join(keepDir(), "logs");
  mkdirSync(logs, { recursive: true });
  copyFileSync(app.log, path.join(logs, path.basename(app.log)));
}

/** The capability key a spec's config sets to ask for an app. */
export const APP_CAPABILITY = "rekindle:app";

/** The session's display (`AppCapability.display`), set by the launcher. */
export const DISPLAY_CAPABILITY = "rekindle:display";

export interface AppCapability {
  /** The window the session starts in (the driver's `wdio:tauriServiceOptions.windowLabel`). */
  window: string;
  /**
   * The display the app runs on, on Linux; elsewhere it changes nothing.
   * `"native"`: the session's own (Wayland or X11). `"x11"`: X11, through
   * XWayland on a Wayland session (`GDK_BACKEND=x11`), for specs that size
   * windows. Wayland gives a client no way to resize or place its own
   * window (xdg-shell: xdg_toplevel has no such request, its configure size
   * is a hint, and resize needs a user action's serial), so `sizeWindow`
   * cannot work there; a spec of how the web content lays out at a size
   * asks nothing of the display protocol, and the windowing behaviour that
   * is Wayland's own is checked natively (specs/native/, plan E7.14 and
   * WL2, 2026-11-20).
   */
  display: "native" | "x11";
  /**
   * More environment for the app: a debug build's test knob a spec's whole
   * config needs (`REKINDLE_ROUTE_BUDGET_CAPACITY`, wdio.budget.conf.ts).
   */
  env?: Record<string, string>;
}

/**
 * Every app's idle (plan AA1): OS input idle is machine-wide and WebDriver's
 * presses never reset it, so on an untouched machine auto-away fired at
 * unlock and raced each spec. Every app runs at idle 0 (the webdriver-only
 * `REKINDLE_WDIO_IDLE_SECONDS`); a spec that needs real idle sets its own
 * value in its capability's `env` (wdio.idle.conf.ts), which wins.
 */
const DEFAULT_TEST_ENV: Record<string, string> = { REKINDLE_WDIO_IDLE_SECONDS: "0" };

/** The environment `caps`' app launches with (`AppCapability.display`, `.env`). */
function displayEnv(where: string, caps: AppCaps): Record<string, string> {
  const wanted = caps[APP_CAPABILITY];
  if (!wanted) throw new Error(`${where}: capability "${APP_CAPABILITY}" is missing`);
  return {
    ...DEFAULT_TEST_ENV,
    ...(wanted.display === "x11" && process.platform === "linux" ? { GDK_BACKEND: "x11" } : {}),
    ...(wanted.env ?? {}),
  };
}

/** Connection options the runner reads from a capability (standalone) or an instance (multiremote). */
interface Connection {
  hostname?: string;
  port?: number;
  protocol?: string;
  path?: string;
  headers?: Record<string, string>;
}

type AppCaps = WebdriverIO.Capabilities & { [APP_CAPABILITY]?: AppCapability };

/** One multiremote instance: its connection options beside its capabilities (@wdio/runner deep-merges it over the config). */
type Instance = Connection & { capabilities: AppCaps };

/**
 * The capabilities a session sends, and the connection it uses, for an app
 * asked for by `caps`. The driver reads only `wdio:tauriServiceOptions`;
 * `browserName` is what it reports for both engines (WKWebView and
 * WebKitGTK: `parse_user_agent`, server/handlers/session.rs), so the
 * runner's own lines name the engine.
 */
function connect(where: string, caps: AppCaps, conn: Connection, app: App): void {
  const wanted = caps[APP_CAPABILITY];
  if (!wanted) throw new Error(`${where}: capability "${APP_CAPABILITY}" is missing`);
  delete caps[APP_CAPABILITY];
  // The display the app runs on (`AppCapability.display`), kept for the
  // session as an extension capability: `rekindle:app` itself is consumed
  // here, so a helper that words a refused resize reads this one.
  (caps as Record<string, unknown>)[DISPLAY_CAPABILITY] = wanted.display;
  caps.browserName = "webkit";
  caps["wdio:enforceWebDriverClassic"] = true;
  (caps as Record<string, unknown>)["wdio:tauriServiceOptions"] = { windowLabel: wanted.window };
  conn.hostname = "127.0.0.1";
  conn.port = app.port;
  conn.protocol = "http";
  conn.path = "/";
  conn.headers = { Authorization: `Bearer ${app.token}` };
}

export class RekindleAppLauncher implements Services.ServiceInstance {
  readonly #apps = new Map<string, App[]>();

  /** Builds the debug app with the driver once per run (`REKINDLE_APP_NO_BUILD=1` runs the last build). */
  onPrepare(): void {
    // Cap the earlier runs' evidence, then name this run's folder before any worker forks (keep.ts).
    pruneKept();
    keepDir();
    if (process.env.REKINDLE_APP_NO_BUILD === "1") return;
    try {
      build();
    } catch (err) {
      // An error thrown from onPrepare is logged and the run goes on, on
      // whatever binaries the last build left (round 14: rekindle-node did not
      // compile, and the fleet ran a CLI and daemon of different builds);
      // only a SevereServiceError stops it (@wdio/cli 10.2.0 build/index.js
      // runServiceHook: any other error is logged with "Continue...").
      throw new SevereServiceError(`the app build failed, so nothing is run: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  /**
   * Standalone capabilities get one app; multiremote capabilities
   * (`{ alice: { capabilities }, bob: { capabilities } }`,
   * webdriver.io/docs/multiremote) get one app per instance, each on its own
   * data root, port and token, so each user is its own Host and Veilid node.
   */
  async onWorkerStart(cid: string, workerCaps: WebdriverIO.Capabilities): Promise<void> {
    // `Services.HookFunctions` types the argument as standalone; the runner
    // passes the worker's copy of either shape (@wdio/cli `startInstance`).
    const caps = workerCaps as Capabilities.RequestedStandaloneCapabilities | Capabilities.RequestedMultiRemoteCapabilities;
    if ("alwaysMatch" in caps || "firstMatch" in caps) {
      throw new Error(`worker ${cid}: give "${APP_CAPABILITY}" as plain capabilities, not alwaysMatch/firstMatch`);
    }
    const apps: App[] = [];
    this.#apps.set(cid, apps);
    if (APP_CAPABILITY in caps) {
      const app = await launch(`wdio-${cid}`, displayEnv(`worker ${cid}`, caps as AppCaps));
      apps.push(app);
      connect(`worker ${cid}`, caps as AppCaps, caps as Connection, app);
      return;
    }
    for (const [name, instance] of Object.entries(caps as Record<string, Instance>)) {
      const app = await launch(`wdio-${cid}-${name}`, displayEnv(`worker ${cid} instance ${name}`, instance.capabilities));
      apps.push(app);
      connect(`worker ${cid} instance ${name}`, instance.capabilities, instance, app);
    }
  }

  /**
   * A worker that failed (`exitCode` not 0, webdriver.io's onWorkerEnd)
   * keeps each of its apps' data root and log in
   * `e2e/app/out/failed/<spec>/<run>/<app>/` and says where, so a failure
   * that only the app's own files explain (round 9's "Channel closed"
   * mid-befriend) can still be traced; a passing worker's roots go.
   */
  async onWorkerEnd(cid: string, exitCode: number, specs: string[], _retries: number, signal?: NodeJS.Signals | null): Promise<void> {
    const apps = this.#apps.get(cid) ?? [];
    this.#apps.delete(cid);
    if (exitCode === 0) {
      await Promise.all(apps.map((app) => stopAndKeepLog(app)));
      return;
    }
    const spec = specs.map((s) => path.basename(s).replace(/\.e2e\.ts$/, "")).join("+") || cid;
    const failed = path.join(import.meta.dirname, "out", "failed", spec, path.basename(keepDir()));
    await Promise.all(apps.map((app) => stopAndKeepLog(app, failed)));
    console.log(`[failed] worker ${cid} (${spec}) exited ${exitCode}${signal ? ` by ${signal}` : ""}: each app's data root and log are kept in ${failed}`);
  }

  async onComplete(): Promise<void> {
    await Promise.all([...this.#apps.values()].flat().map((app) => stopAndKeepLog(app)));
    this.#apps.clear();
  }
}
