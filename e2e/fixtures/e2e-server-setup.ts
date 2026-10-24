/**
 * Global setup of the e2e project: start the bridge, wait for the line it
 * prints with its random port and per-run token, and export both to the
 * test workers (`REKINDLE_E2E_URL`, `REKINDLE_E2E_TOKEN`). The returned
 * function is the teardown that stops it.
 */

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

import { FRONTEND_URL } from "../../playwright.base.config";
import { READY_PREFIX, type E2EBridge } from "./e2e-server";

/** Long enough for a cold `cargo run` of the bridge. */
const START_TIMEOUT_MS = 600_000;

export default async function globalSetup(): Promise<() => Promise<void>> {
  const child = spawn(
    "cargo",
    ["run", "-q", "-p", "rekindle-e2e-server", "--bin", "e2e-server"],
    {
      stdio: ["ignore", "pipe", "inherit"],
      // The only browser origin the bridge answers.
      env: { ...process.env, REKINDLE_E2E_ORIGIN: new URL(FRONTEND_URL).origin },
    },
  );
  const bridge = await new Promise<E2EBridge>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("the E2E bridge did not report its address")),
      START_TIMEOUT_MS,
    );
    // Keep draining stdout after the ready line, or the bridge blocks on
    // a full pipe.
    createInterface({ input: child.stdout }).on("line", (line) => {
      const at = line.indexOf(READY_PREFIX);
      if (at >= 0) {
        clearTimeout(timer);
        resolve(JSON.parse(line.slice(at + READY_PREFIX.length)) as E2EBridge);
      }
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`the E2E bridge exited (${code})`));
    });
  });
  process.env.REKINDLE_E2E_URL = bridge.url;
  process.env.REKINDLE_E2E_TOKEN = bridge.token;
  return async () => {
    child.kill();
  };
}
