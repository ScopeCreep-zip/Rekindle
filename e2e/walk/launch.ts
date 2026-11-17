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
}

async function waitForStatus(app: App): Promise<void> {
  const deadline = Date.now() + STATUS_TIMEOUT_MS;
  let last = "no answer";
  while (Date.now() < deadline) {
    if (!app.alive()) throw new Error(`app exited before /status answered (see ${path.join(OUT_DIR, "app.log")})`);
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

export async function launch(): Promise<App> {
  if (!existsSync(BINARY)) throw new Error(`${BINARY} is missing; run without --no-build`);
  mkdirSync(OUT_DIR, { recursive: true });
  const dataRoot = mkdtempSync(path.join(tmpdir(), "rekindle-walk-"));
  const port = await freePort();
  const token = base32Lower(randomBytes(32));
  const log = createWriteStream(path.join(OUT_DIR, "app.log"));
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
      rmSync(dataRoot, { recursive: true, force: true });
    },
  };

  const onSignal = (): void => {
    void app.stop().finally(() => process.exit(130));
  };
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  try {
    await waitForStatus(app);
  } catch (err) {
    await app.stop();
    throw err;
  }
  return app;
}
