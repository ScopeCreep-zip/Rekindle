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

import type { Capabilities, Services } from "@wdio/types";
import { build, launch, type App } from "../walk/launch.ts";

/** The capability key a spec's config sets to ask for an app. */
export const APP_CAPABILITY = "rekindle:app";

export interface AppCapability {
  /** The window the session starts in (the driver's `wdio:tauriServiceOptions.windowLabel`). */
  window: string;
}

type WorkerCaps = WebdriverIO.Capabilities & {
  [APP_CAPABILITY]?: AppCapability;
  hostname?: string;
  port?: number;
  protocol?: string;
  path?: string;
  headers?: Record<string, string>;
};

export class RekindleAppLauncher implements Services.ServiceInstance {
  readonly #apps = new Map<string, App>();

  /** Builds the debug app with the driver once per run (`REKINDLE_APP_NO_BUILD=1` runs the last build). */
  onPrepare(): void {
    if (process.env.REKINDLE_APP_NO_BUILD !== "1") build();
  }

  async onWorkerStart(cid: string, caps: Capabilities.RequestedStandaloneCapabilities): Promise<void> {
    if ("alwaysMatch" in caps || "firstMatch" in caps) {
      throw new Error(`worker ${cid}: give "${APP_CAPABILITY}" as plain capabilities, not alwaysMatch/firstMatch`);
    }
    const worker = caps as WorkerCaps;
    const wanted = worker[APP_CAPABILITY];
    if (!wanted) throw new Error(`worker ${cid}: capability "${APP_CAPABILITY}" is missing`);
    const app = await launch(`wdio-${cid}`);
    this.#apps.set(cid, app);
    delete worker[APP_CAPABILITY];
    worker.hostname = "127.0.0.1";
    worker.port = app.port;
    worker.protocol = "http";
    worker.path = "/";
    worker.headers = { Authorization: `Bearer ${app.token}` };
    worker["wdio:enforceWebDriverClassic"] = true;
    (worker as Record<string, unknown>)["wdio:tauriServiceOptions"] = { windowLabel: wanted.window };
  }

  async onWorkerEnd(cid: string): Promise<void> {
    const app = this.#apps.get(cid);
    this.#apps.delete(cid);
    await app?.stop();
  }

  async onComplete(): Promise<void> {
    await Promise.all([...this.#apps.values()].map((app) => app.stop()));
    this.#apps.clear();
  }
}
