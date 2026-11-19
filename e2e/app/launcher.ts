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
    if (process.env.REKINDLE_APP_NO_BUILD !== "1") build();
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
      const app = await launch(`wdio-${cid}`);
      apps.push(app);
      connect(`worker ${cid}`, caps as AppCaps, caps as Connection, app);
      return;
    }
    for (const [name, instance] of Object.entries(caps as Record<string, Instance>)) {
      const app = await launch(`wdio-${cid}-${name}`);
      apps.push(app);
      connect(`worker ${cid} instance ${name}`, instance.capabilities, instance, app);
    }
  }

  async onWorkerEnd(cid: string): Promise<void> {
    const apps = this.#apps.get(cid) ?? [];
    this.#apps.delete(cid);
    await Promise.all(apps.map((app) => app.stop()));
  }

  async onComplete(): Promise<void> {
    await Promise.all([...this.#apps.values()].flat().map((app) => app.stop()));
    this.#apps.clear();
  }
}
