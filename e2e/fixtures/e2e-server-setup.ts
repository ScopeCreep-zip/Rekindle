/**
 * Global setup of the e2e project: make a private directory for this run,
 * start the bridge with the path of a handshake file in it, wait for the
 * bridge to write its random port and per-run token there (owner-only,
 * never on stdout, so the token stays out of CI logs), and export both to
 * the test workers (`REKINDLE_E2E_URL`, `REKINDLE_E2E_TOKEN`). The returned
 * function is the teardown that stops the bridge and removes the directory.
 */

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { FRONTEND_URL } from "../../playwright.base.config";
import { HANDSHAKE_ENV, type E2EBridge } from "./e2e-server";

/** Long enough for a cold `cargo run` of the bridge. */
const START_TIMEOUT_MS = 600_000;

/** How often to look for the handshake file. */
const POLL_MS = 200;

export default async function globalSetup(): Promise<() => Promise<void>> {
  // mkdtemp creates the directory 0700: only this user can list or read it.
  const dir = mkdtempSync(join(tmpdir(), "rekindle-e2e-"));
  const handshake = join(dir, "bridge.json");
  const child = spawn(
    "cargo",
    ["run", "-q", "-p", "rekindle-e2e-server", "--features", "e2e-bridge", "--bin", "e2e-server"],
    {
      stdio: ["ignore", "inherit", "inherit"],
      env: {
        ...process.env,
        // The only browser origin the bridge answers.
        REKINDLE_E2E_ORIGIN: new URL(FRONTEND_URL).origin,
        [HANDSHAKE_ENV]: handshake,
      },
    },
  );
  const stop = async () => {
    child.kill();
    rmSync(dir, { recursive: true, force: true });
  };
  try {
    const bridge = await new Promise<E2EBridge>((resolve, reject) => {
      const started = Date.now();
      const onExit = (code: number | null) => reject(new Error(`the E2E bridge exited (${code})`));
      child.on("exit", onExit);
      const poll = setInterval(() => {
        // The bridge renames the file into place whole, so it is complete
        // once it exists.
        if (existsSync(handshake)) {
          clearInterval(poll);
          child.off("exit", onExit);
          resolve(JSON.parse(readFileSync(handshake, "utf8")) as E2EBridge);
        } else if (Date.now() - started > START_TIMEOUT_MS) {
          clearInterval(poll);
          reject(new Error("the E2E bridge did not report its address"));
        }
      }, POLL_MS);
    });
    process.env.REKINDLE_E2E_URL = bridge.url;
    process.env.REKINDLE_E2E_TOKEN = bridge.token;
  } catch (e) {
    await stop();
    throw e;
  }
  return stop;
}
