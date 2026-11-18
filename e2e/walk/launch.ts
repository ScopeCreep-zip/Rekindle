// Plan E7.13: builds and launches the desktop app with its embedded
// WebDriver server (cargo feature `webdriver-test`, debug only), and tears
// it down. The server admits a request only with `Host: 127.0.0.1:<port>`,
// no `Origin`, and `Authorization: Bearer <token>`
// (crates/vendor/tauri-plugin-wdio-webdriver/src/server/auth.rs).

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { remote, type Browser } from "webdriverio";

export const REPO_ROOT = path.resolve(import.meta.dirname, "..", "..");
export const OUT_DIR = path.join(REPO_ROOT, "e2e", "walk", "out");
const BINARY = path.join(REPO_ROOT, "target", "debug", process.platform === "win32" ? "rekindle-desktop.exe" : "rekindle-desktop");
const STATUS_TIMEOUT_MS = 180_000;

export interface App {
  port: number;
  /** Never printed or written anywhere but the child's environment. */
  token: string;
  dataRoot: string;
  child: ChildProcess;
  alive(): boolean;
  stop(): Promise<void>;
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/** RFC 4648 base32, no padding, lowercased: the spelling `BearerToken::parse` accepts. */
function base32Lower(bytes: Uint8Array): string {
  let out = "";
  let bits = 0;
  let value = 0;
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += BASE32[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31];
  return out.toLowerCase();
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address !== null ? address.port : 0;
      server.close(() => (port > 0 ? resolve(port) : reject(new Error("no free port"))));
    });
  });
}

export function build(): void {
  console.log("[walk] building: pnpm tauri build --debug --no-bundle --features webdriver-test");
  const result = spawnSync("pnpm", ["tauri", "build", "--debug", "--no-bundle", "--features", "webdriver-test"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  if (result.status !== 0) throw new Error(`build failed (exit ${result.status ?? result.signal})`);
  // The Host runs every skin package through rekindle-skin-helper beside its
  // own executable (rekindle-skin helper/mod.rs:98 beside_current_exe);
  // without it ST12's import was refused as "helper not found", not by the
  // helper's check of the package.
  console.log("[walk] building: cargo build -p rekindle-skin --bin rekindle-skin-helper");
  const helper = spawnSync("cargo", ["build", "-p", "rekindle-skin", "--bin", "rekindle-skin-helper"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  if (helper.status !== 0) throw new Error(`helper build failed (exit ${helper.status ?? helper.signal})`);
}

async function waitForStatus(app: App, logName: string): Promise<void> {
  const deadline = Date.now() + STATUS_TIMEOUT_MS;
  let last = "no answer";
  while (Date.now() < deadline) {
    if (!app.alive()) throw new Error(`app exited before /status answered (see ${path.join(OUT_DIR, logName)})`);
    try {
      const res = await fetch(`http://127.0.0.1:${app.port}/status`, {
        headers: { Authorization: `Bearer ${app.token}` },
      });
      if (res.ok) {
        const body = (await res.json()) as { value?: { ready?: boolean } };
        if (body.value?.ready !== false) return;
        last = "ready=false";
      } else {
        last = `HTTP ${res.status}`;
      }
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`WebDriver /status not ready after ${STATUS_TIMEOUT_MS} ms (${last})`);
}

const running = new Set<App>();
let signalsHooked = false;

/**
 * Launch one app on its own temp data root, port and token. The walk may
 * run two (a second identity for the [PEER] rows): the webdriver build has
 * no single-instance plugin (lib.rs:109), the node lock is per data root
 * (setup.rs `host::claim`), and Veilid's empty listen address searches
 * upward from 5150 (veilid-core start_protocols.rs:89-95), so they coexist.
 */
export async function launch(name = "app"): Promise<App> {
  if (!existsSync(BINARY)) throw new Error(`${BINARY} is missing; run without --no-build`);
  mkdirSync(OUT_DIR, { recursive: true });
  const dataRoot = mkdtempSync(path.join(tmpdir(), "rekindle-walk-"));
  console.log(`[walk] ${name} data root: ${dataRoot}`);
  const port = await freePort();
  const token = base32Lower(randomBytes(32));
  const logName = `${name}.log`;
  const log = createWriteStream(path.join(OUT_DIR, logName));
  const child = spawn(BINARY, [], {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      REKINDLE_WDIO_DATA_ROOT: dataRoot,
      TAURI_WEBDRIVER_PORT: String(port),
      REKINDLE_WDIO_TOKEN: token,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout?.pipe(log);
  child.stderr?.pipe(log);
  let exited = false;
  child.once("exit", () => {
    exited = true;
    // A crash (a peer panicking at startup) ends the child with no stop()
    // from the walk; its root holds nothing the walk reads (the log is in
    // OUT_DIR), so it goes as soon as nothing writes to it.
    rmSync(dataRoot, { recursive: true, force: true });
  });

  let stopped = false;
  const app: App = {
    port,
    token,
    dataRoot,
    child,
    alive: () => !exited,
    async stop() {
      if (stopped) return;
      stopped = true;
      if (!exited) {
        child.kill("SIGTERM");
        const gone = await Promise.race([
          new Promise<boolean>((r) => child.once("exit", () => r(true))),
          new Promise<boolean>((r) => setTimeout(() => r(false), 5000)),
        ]);
        if (!gone && !exited) child.kill("SIGKILL");
      }
      log.end();
      running.delete(app);
      rmSync(dataRoot, { recursive: true, force: true });
    },
  };
  running.add(app);

  if (!signalsHooked) {
    signalsHooked = true;
    const onSignal = (): void => {
      void Promise.all([...running].map((a) => a.stop())).finally(() => process.exit(130));
    };
    process.once("SIGINT", onSignal);
    process.once("SIGTERM", onSignal);
    // Every other way out (process.exit after the table, an abort, an
    // uncaught error, a launch a timed-out row left running) ends here,
    // where only synchronous work runs: kill what is left, drop its root.
    process.once("exit", () => {
      for (const a of running) {
        if (a.alive()) a.child.kill("SIGKILL");
        rmSync(a.dataRoot, { recursive: true, force: true });
      }
    });
  }

  try {
    await waitForStatus(app, logName);
  } catch (err) {
    await app.stop();
    throw err;
  }
  return app;
}

/**
 * A new WebDriver session on `app`, starting in window `windowLabel`. The
 * server keeps sessions apart (vendor `webdriver/session.rs`
 * `SessionManager`), so deleting one makes every later command from it fail
 * with "invalid session id": that is how a row past its deadline is stopped.
 */
export function openSession(app: App, windowLabel: string): Promise<Browser> {
  return remote({
    hostname: "127.0.0.1",
    port: app.port,
    path: "/",
    protocol: "http",
    logLevel: "error",
    connectionRetryCount: 1,
    headers: { Authorization: `Bearer ${app.token}` },
    capabilities: {
      "wdio:enforceWebDriverClassic": true,
      "wdio:tauriServiceOptions": { windowLabel },
    } as WebdriverIO.Capabilities,
  });
}

/**
 * End `old` and open a fresh session on a live window (buddy list first,
 * then login). Commands a cancelled row still has in flight land on the
 * ended session and fail instead of driving the app.
 */
export async function renewSession(app: App, old: Browser): Promise<Browser> {
  const handles = await old.getWindowHandles().catch(() => [] as string[]);
  await old.deleteSession().catch(() => undefined);
  const label = handles.find((h) => h === "buddy-list") ?? handles.find((h) => h === "login") ?? handles[0];
  return openSession(app, label ?? "login");
}
