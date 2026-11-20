// Plan E7.13: builds and launches the desktop app with its embedded
// WebDriver server (cargo feature `webdriver-test`, debug only), and tears
// it down. The server admits a request only with `Host: 127.0.0.1:<port>`,
// no `Origin`, and `Authorization: Bearer <token>`
// (crates/vendor/tauri-plugin-wdio-webdriver/src/server/auth.rs).

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";
import { cpSync, createWriteStream, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { remote, type Browser } from "webdriverio";

export const REPO_ROOT = path.resolve(import.meta.dirname, "..", "..");
export const OUT_DIR = path.join(REPO_ROOT, "e2e", "walk", "out");
/**
 * Arguments for a launched app. On macOS, `-NSAppSleepDisabled YES` puts
 * `NSAppSleepDisabled` in Cocoa's argument domain (NSUserDefaults reads
 * `-key value` pairs from argv), so App Nap never throttles a test app
 * whose windows sit behind others on a desktop in use: layout run 4 lost
 * two tests to WebKit script timeouts (`evaluateJavaScript` unanswered for
 * the whole script timeout, macos.rs `evaluate_js`) after minutes occluded.
 * The app parses no argv of its own (the deep-link plugin reads only
 * `rekindle://` URLs).
 */
function appArgs(): string[] {
  return process.platform === "darwin" ? ["-NSAppSleepDisabled", "YES"] : [];
}

let binary: string | undefined;
let targetDirectory: string | undefined;

/**
 * Cargo's target directory as cargo resolves it (`CARGO_TARGET_DIR`, then
 * `build.target-dir`, then `<workspace>/target`; `cargo metadata`'s
 * `target_directory`). `build()` and `tauri build` inherit the same
 * environment, so a run launches the binaries its build wrote, also when a
 * gate builds in its own target directory.
 */
export function cargoTargetDir(): string {
  if (targetDirectory) return targetDirectory;
  const meta = spawnSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (meta.status !== 0) throw new Error(`cargo metadata failed (exit ${meta.status ?? meta.signal}): ${meta.stderr}`);
  targetDirectory = (JSON.parse(meta.stdout) as { target_directory: string }).target_directory;
  return targetDirectory;
}

/** The debug app in cargo's target directory. */
function appBinary(): string {
  binary ??= path.join(cargoTargetDir(), "debug", process.platform === "win32" ? "rekindle-desktop.exe" : "rekindle-desktop");
  return binary;
}

const STATUS_TIMEOUT_MS = 180_000;

export interface App {
  port: number;
  /** Never printed or written anywhere but the child's environment. */
  token: string;
  dataRoot: string;
  /** The app's stdout and stderr, in OUT_DIR; a later launch of the same name overwrites it. */
  log: string;
  child: ChildProcess;
  alive(): boolean;
  /**
   * Stop the app and drop its data root. With `keep`, the root and the log
   * are copied to `keep/data-root` and `keep/<log>` first, so a failure can
   * be traced after the run (the app's own files: its database, vault and
   * Veilid storage of a throwaway identity, never the token).
   */
  stop(keep?: string): Promise<void>;
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
  // Plan E7.14's peer fleet (e2e/app/fleet.ts): headless peers are
  // `rekindled` driven by the `rekindle` CLI.
  console.log("[walk] building: cargo build -p rekindle-node --bin rekindled -p rekindle-cli --bin rekindle");
  const fleet = spawnSync("cargo", ["build", "-p", "rekindle-node", "--bin", "rekindled", "-p", "rekindle-cli", "--bin", "rekindle"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
  if (fleet.status !== 0) throw new Error(`fleet build failed (exit ${fleet.status ?? fleet.signal})`);
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
export async function launch(name = "app", env: Record<string, string> = {}): Promise<App> {
  const exe = appBinary();
  if (!existsSync(exe)) throw new Error(`${exe} is missing; build it first (pnpm walk without --no-build, or pnpm test:app without REKINDLE_APP_NO_BUILD)`);
  mkdirSync(OUT_DIR, { recursive: true });
  const dataRoot = mkdtempSync(path.join(tmpdir(), "rekindle-walk-"));
  console.log(`[walk] ${name} data root: ${dataRoot}`);
  const port = await freePort();
  const token = base32Lower(randomBytes(32));
  const logName = `${name}.log`;
  const log = createWriteStream(path.join(OUT_DIR, logName));
  const child = spawn(exe, appArgs(), {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      ...env,
      REKINDLE_WDIO_DATA_ROOT: dataRoot,
      TAURI_WEBDRIVER_PORT: String(port),
      REKINDLE_WDIO_TOKEN: token,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  // Neither stream ends the log (a pipe ends its destination when its source
  // ends, and the log outlives the child's streams): stop() ends it.
  child.stdout?.pipe(log, { end: false });
  child.stderr?.pipe(log, { end: false });
  let exited = false;
  let stopped = false;
  child.once("exit", () => {
    exited = true;
  });
  // `close` comes once the child's streams are drained, so the line lands
  // after the app's own last lines. An exit nobody asked for (a crash, a
  // panic) is written into the log; the root stays until stop() or the
  // process exit, so a failed run can keep it (round 9's "Channel closed"
  // mid-test was undiagnosable once the root was gone).
  const closed = new Promise<void>((resolve) => {
    child.once("close", (code, signal) => {
      if (!stopped) log.write(`\n[walk] the app exited by itself: code ${code ?? "none"}, signal ${signal ?? "none"}\n`);
      resolve();
    });
  });

  const app: App = {
    port,
    token,
    dataRoot,
    log: path.join(OUT_DIR, logName),
    child,
    alive: () => !exited,
    async stop(keep?: string) {
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
      // The child's last output is in before the log ends (a write after the
      // end would be an unhandled error); a stream a helper process still
      // holds open does not hold the stop longer than 5 s.
      await Promise.race([closed, new Promise<void>((r) => setTimeout(r, 5000))]);
      child.stdout?.unpipe(log);
      child.stderr?.unpipe(log);
      // Flushed before stop() resolves, so a caller can copy the log whole.
      await new Promise<void>((resolve) => log.end(resolve));
      running.delete(app);
      if (keep !== undefined) {
        mkdirSync(keep, { recursive: true });
        cpSync(dataRoot, path.join(keep, "data-root"), { recursive: true });
        cpSync(path.join(OUT_DIR, logName), path.join(keep, logName));
      }
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
